use qcom_rproc::{
    glink::{Header, Link, Ring},
    pas::{Pas, PasId, Request, Smc},
    qmi::{Message, Tlv},
    qrtr::{Nameserver, Packet},
    smem::{Global, Model},
    Error,
};
#[derive(Default)]
struct Mock {
    calls: Vec<Request>,
    status: u64,
}
impl Smc for Mock {
    fn call(&mut self, r: Request) -> Result<u64, i64> {
        self.calls.push(r);
        Ok(self.status)
    }
}
#[test]
fn pas_requests_and_negative_ids() {
    assert_eq!(PasId::new(0, &[1]), Err(Error::InvalidId));
    assert_eq!(PasId::new(9, &[1]), Err(Error::InvalidId));
    let id = PasId::new(1, &[1]).unwrap();
    let mut p = Pas::new(Mock::default());
    assert_eq!(p.init_image(id, 3), Err(Error::InvalidAddress));
    p.init_image(id, 0x1000).unwrap();
    p.mem_setup(id, 0x2000, 0x1000).unwrap();
    p.auth_and_reset(id).unwrap();
    p.shutdown(id).unwrap();
    let calls = p.into_inner().calls;
    assert_eq!(
        calls.iter().map(|r| r.function & 0xff).collect::<Vec<_>>(),
        [1, 2, 5, 6]
    );
    assert_eq!(calls[0].arginfo, 0x82);
    assert_eq!(calls[1].args, [1, 0x2000, 0x1000]);
    let mut bad = Pas::new(Mock {
        status: 5,
        ..Mock::default()
    });
    assert_eq!(bad.auth_and_reset(id), Err(Error::Smc(5)));
}
#[test]
fn smem_bounds_and_missing() {
    let mut m = Model::default();
    assert_eq!(m.get(2, 429), Err(Error::MissingItem));
    m.insert(2, 429, vec![1, 2]).unwrap();
    assert_eq!(m.get(2, 429).unwrap(), [1, 2]);
    assert_eq!(m.insert(2, 429, vec![9]), Err(Error::Duplicate));
    assert_eq!(m.get(2, 429).unwrap(), [1, 2]);
    let mut image = vec![0; 9000];
    let at = 208 + 429 * 16;
    image[at..at + 4].copy_from_slice(&1u32.to_le_bytes());
    image[at + 4..at + 8].copy_from_slice(&8000u32.to_le_bytes());
    image[at + 8..at + 12].copy_from_slice(&2u32.to_le_bytes());
    image[8000..8002].copy_from_slice(&[3, 4]);
    assert_eq!(Global::new(&image, 0).get(429).unwrap(), [3, 4]);
    assert_eq!(Global::new(&image[..4], 0).get(429), Err(Error::Bounds));
    image[at + 12..at + 16].copy_from_slice(&0x1000u32.to_le_bytes());
    assert_eq!(Global::new(&image, 0).get(429), Err(Error::MissingItem));
    image[at + 12..at + 16].copy_from_slice(&0u32.to_le_bytes());
    image[at + 4..at + 8].copy_from_slice(&9000u32.to_le_bytes());
    assert_eq!(Global::new(&image, 0).get(429), Err(Error::Bounds));
}
#[test]
fn glink_ring_channel_intents() {
    let mut ring = Ring::new(10);
    ring.write(&[1; 8]).unwrap();
    assert_eq!(ring.write(&[2; 3]), Err(Error::Full));
    assert_eq!(ring.read(9), Err(Error::Bounds));
    assert_eq!(ring.read(8).unwrap(), [1; 8]);
    let mut link = Link::new();
    let open = link.open(1, "apr_audio_svc").unwrap();
    assert_eq!(Header::decode(&open.encode()).unwrap(), open);
    assert_eq!(link.open(2, "apr_audio_svc"), Err(Error::Duplicate));
    assert_eq!(link.send(1, b"hi"), Err(Error::NoIntent));
    link.advertise_remote_intent(1, 7, 4).unwrap();
    assert_eq!(link.send(1, b"12345"), Err(Error::NoIntent));
    assert_eq!(link.send(1, b"hi").unwrap().1, 7);
    assert_eq!(link.send(1, b"x"), Err(Error::NoIntent));
    link.close(1).unwrap();
}
#[test]
fn qrtr_vector_and_nameserver() {
    let p = Packet {
        kind: 1,
        src_node: 2,
        src_port: 3,
        confirm_rx: false,
        dst_node: 4,
        dst_port: 5,
        payload: vec![0xaa],
    };
    let wire = p.encode().unwrap();
    assert_eq!(&wire[..8], &[1, 0, 0, 0, 1, 0, 0, 0]);
    assert_eq!(Packet::decode(&wire).unwrap(), p);
    assert_eq!(Packet::decode(&wire[..31]), Err(Error::Malformed));
    let mut ns = Nameserver::default();
    ns.register(42, 1, 4, 5).unwrap();
    assert_eq!(ns.register(42, 1, 6, 7), Err(Error::Duplicate));
    assert_eq!(ns.lookup(42, 1).unwrap(), (4, 5));
    ns.bind(5, 99).unwrap();
    assert_eq!(ns.route(&p), Ok(99));
}
#[test]
fn qmi_golden_tlv_and_malformed() {
    let m = Message {
        kind: 0,
        transaction: 0x1234,
        message: 0x20,
        tlvs: vec![Tlv {
            kind: 1,
            value: vec![0xaa, 0xbb],
        }],
    };
    let golden = [0, 0x34, 0x12, 0x20, 0, 5, 0, 1, 2, 0, 0xaa, 0xbb];
    assert_eq!(m.encode().unwrap(), golden);
    assert_eq!(Message::decode(&golden).unwrap(), m);
    assert_eq!(m.required(1).unwrap(), [0xaa, 0xbb]);
    assert_eq!(m.required(2), Err(Error::MissingItem));
    assert_eq!(Message::decode(&golden[..11]), Err(Error::Malformed));
    let mut bad = golden;
    bad[8] = 3;
    assert_eq!(Message::decode(&bad), Err(Error::Malformed));
    let dup = Message {
        tlvs: vec![m.tlvs[0].clone(), m.tlvs[0].clone()],
        ..m
    };
    assert_eq!(dup.required(1), Err(Error::Duplicate));
}
#[test]
fn glink_version_and_frame_vector() {
    use qcom_rproc::glink::{data_frame, negotiate_version, parse_data_frame};
    let ack = negotiate_version(Header {
        command: 0,
        param1: 1,
        param2: 3,
    })
    .unwrap();
    assert_eq!(
        ack,
        Header {
            command: 1,
            param1: 1,
            param2: 1
        }
    );
    assert_eq!(
        negotiate_version(Header {
            command: 0,
            param1: 2,
            param2: 0
        }),
        Err(Error::Malformed)
    );
    let wire = data_frame(2, 7, b"abc").unwrap();
    assert_eq!(
        &wire[..16],
        &[9, 0, 2, 0, 7, 0, 0, 0, 3, 0, 0, 0, 0, 0, 0, 0]
    );
    assert_eq!(parse_data_frame(&wire).unwrap(), (2, 7, b"abc".as_slice()));
    let mut bad = wire.clone();
    bad[12] = 1;
    assert_eq!(parse_data_frame(&bad), Err(Error::Malformed));
    bad = wire;
    bad[19] = 1;
    assert_eq!(parse_data_frame(&bad), Err(Error::Malformed));
}
#[test]
fn pas_invalid_memory_never_calls_smc() {
    let id = PasId::new(7, &[7]).unwrap();
    let mut p = Pas::new(Mock::default());
    assert_eq!(p.mem_setup(id, 0, 4096), Err(Error::InvalidAddress));
    assert_eq!(p.mem_setup(id, 4096, 0), Err(Error::InvalidAddress));
    assert_eq!(
        p.mem_setup(id, u64::MAX - 4095, 8192),
        Err(Error::InvalidAddress)
    );
    assert!(p.into_inner().calls.is_empty());
}
#[test]
fn qmi_rejects_trailing_and_invalid_type() {
    let bytes = [0, 1, 0, 2, 0, 0, 0];
    let mut extra = bytes.to_vec();
    extra.push(0);
    assert_eq!(Message::decode(&extra), Err(Error::Malformed));
    let mut invalid = bytes;
    invalid[0] = 1;
    assert_eq!(Message::decode(&invalid), Err(Error::Malformed));
}
