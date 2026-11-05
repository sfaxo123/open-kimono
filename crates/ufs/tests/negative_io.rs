//! Transfer, SCSI, refusal and GPT failure paths: typed errors, no slot left outstanding.

mod common;

use common::model::UfsModel;
use common::{config, dma, phone_device, rig, SharedDma, BASE};
use mmio_replay::{Replayer, Trace};
use ufs_core::gpt::GptError;
use ufs_core::regs;
use ufs_core::scsi::{Cmd, Sense};
use ufs_core::{Error, NoDelay, Stage, Ufshc};

fn up(m: &mut UfsModel, d: SharedDma) -> Ufshc<&mut UfsModel, SharedDma, NoDelay> {
    let mut hc = Ufshc::new(m, BASE, d, NoDelay, config()).unwrap();
    hc.bring_up().unwrap();
    hc
}

#[test]
fn ocs_error_on_a_command() {
    let (mut m, d) = rig(phone_device());
    up(&mut m, d.clone());
    m.dev.faults.command_ocs = Some(0x08);
    let mut hc = Ufshc::new(&mut m, BASE, d, NoDelay, config()).unwrap();
    assert_eq!(hc.read_capacity(0), Err(Error::Ocs(0x08)));
}

#[test]
fn doorbell_timeout_clears_the_slot_and_recovers() {
    let (mut m, d) = rig(phone_device());
    up(&mut m, d.clone());
    m.hw.hang_doorbell = true;
    let mut hc = Ufshc::new(&mut m, BASE, d.clone(), NoDelay, config()).unwrap();
    assert_eq!(hc.read_capacity(0), Err(Error::Timeout(Stage::Doorbell)));
    drop(hc);
    assert_eq!(m.dbr, 0, "UTRLCLR (0 bit) released slot 0");
    m.hw.hang_doorbell = false;
    let mut hc = Ufshc::new(&mut m, BASE, d, NoDelay, config()).unwrap();
    assert_eq!(
        hc.read_capacity(0).unwrap().last_lba,
        63,
        "next command succeeds"
    );
}

#[test]
fn slot_that_ignores_clear_times_out_in_clear() {
    let (mut m, d) = rig(phone_device());
    up(&mut m, d.clone());
    m.hw.hang_doorbell = true;
    m.hw.clear_ignored = true;
    let mut hc = Ufshc::new(&mut m, BASE, d, NoDelay, config()).unwrap();
    assert_eq!(hc.read_capacity(0), Err(Error::Timeout(Stage::Clear)));
}

#[test]
fn fatal_interrupt_status_aborts_the_transfer() {
    let (mut m, d) = rig(phone_device());
    up(&mut m, d.clone());
    m.hw.fatal_is = Some(regs::IS_HCFES);
    let mut hc = Ufshc::new(&mut m, BASE, d, NoDelay, config()).unwrap();
    assert_eq!(hc.read_capacity(0), Err(Error::Fatal(regs::IS_HCFES)));
    drop(hc);
    assert_eq!(m.dbr, 0);
}

#[test]
fn scsi_check_conditions() {
    let mut dev = phone_device();
    dev.faults.read_error = Some((0x03, 0x11));
    let (mut m, d) = rig(dev);
    let mut hc = up(&mut m, d);
    let mut b = vec![0u8; 4096];
    let medium = Error::CheckCondition(Sense {
        key: 3,
        asc: 0x11,
        ascq: 0,
    });
    assert_eq!(hc.read(0, 0, 4096, &mut b), Err(medium));
    // Unsupported / disabled LU → ILLEGAL REQUEST, LOGICAL UNIT NOT SUPPORTED.
    let lu = Error::CheckCondition(Sense {
        key: 5,
        asc: 0x25,
        ascq: 0,
    });
    assert_eq!(hc.read_capacity(2), Err(lu));
}

#[test]
fn read_past_the_end_of_a_lu() {
    let (mut m, d) = rig(phone_device());
    let mut hc = up(&mut m, d);
    let mut b = vec![0u8; 2 * 4096];
    let e = Error::CheckCondition(Sense {
        key: 5,
        asc: 0x21,
        ascq: 0,
    });
    assert_eq!(hc.read(0, 63, 4096, &mut b), Err(e));
}

#[test]
fn short_data_in_is_reported() {
    let (mut m, d) = rig(phone_device());
    up(&mut m, d.clone());
    m.dev.faults.residual = Some(4);
    let mut hc = Ufshc::new(&mut m, BASE, d, NoDelay, config()).unwrap();
    assert_eq!(hc.read_capacity(0), Err(Error::ShortTransfer(4)));
}

#[test]
fn mismatched_task_tag_is_rejected() {
    let (mut m, d) = rig(phone_device());
    up(&mut m, d.clone());
    m.dev.faults.bad_tag = true;
    let mut hc = Ufshc::new(&mut m, BASE, d, NoDelay, config()).unwrap();
    assert_eq!(hc.read_capacity(0), Err(Error::BadUpiu));
}

#[test]
fn query_errors_surface_the_response_code() {
    let (mut m, d) = rig(phone_device());
    let mut hc = up(&mut m, d);
    assert_eq!(hc.read_descriptor(0x55, 0), Err(Error::Response(0xfd)));
    assert_eq!(hc.read_descriptor(0x02, 8), Err(Error::Response(0xfc)));
}

/// A replayer with an empty trace fails on the first access: proves "nothing was sent".
fn silent() -> Ufshc<Replayer, SharedDma, NoDelay> {
    Ufshc::new(
        Replayer::strict(Trace::new()),
        BASE,
        dma(),
        NoDelay,
        config(),
    )
    .unwrap()
}

#[test]
fn write_class_opcodes_are_refused_without_any_access() {
    let mut hc = silent();
    let mut buf = [0u8; 4096];
    // WRITE(6/10/16), WRITE SAME, SYNC CACHE, UNMAP, START STOP, MODE SELECT, FORMAT, WRITE BUFFER.
    for op in [
        0x0a, 0x2a, 0x8a, 0x41, 0x35, 0x42, 0x1b, 0x55, 0x04, 0x3b, 0xa0,
    ] {
        assert_eq!(
            hc.scsi_raw(0, &[op, 0, 0, 0, 0, 0, 0, 0, 1, 0], 4096, &mut buf),
            Err(Error::Refused(op))
        );
    }
    assert!(hc.into_parts().0.finish().ok());
}

#[test]
fn invalid_arguments_make_no_access() {
    let mut hc = silent();
    let mut buf = [0u8; 4096];
    assert_eq!(
        hc.scsi_raw(0x81, &[0x00], 0, &mut buf),
        Err(Error::InvalidArgument),
        "W-LUN"
    );
    assert_eq!(
        hc.scsi_raw(0, &[], 0, &mut buf),
        Err(Error::InvalidArgument)
    );
    assert_eq!(
        hc.scsi_raw(0, &[0x28; 17], 0, &mut buf),
        Err(Error::InvalidArgument)
    );
    assert_eq!(
        hc.scsi(0, Cmd::Read10 { lba: 0, blocks: 2 }, 4096, &mut buf),
        Err(Error::InvalidArgument)
    );
    assert_eq!(
        hc.read(0, 0, 4096, &mut buf[..100]),
        Err(Error::InvalidArgument)
    );
    assert_eq!(
        hc.read(0, 0, 0, &mut buf),
        Err(Error::InvalidArgument),
        "zero block size"
    );
    let past = u32::MAX as u64 + 1;
    assert_eq!(
        hc.read(0, past, 4096, &mut buf),
        Err(Error::InvalidArgument),
        "beyond READ(10) LBA range"
    );
    let big = 0x2_0000 + 4096;
    let mut huge = vec![0u8; big];
    assert_eq!(
        hc.scsi(
            0,
            Cmd::Read10 {
                lba: 0,
                blocks: (big / 4096) as u16
            },
            4096,
            &mut huge
        ),
        Err(Error::InvalidArgument),
        "beyond DMA buffer"
    );
    assert!(hc.into_parts().0.finish().ok());
}

#[test]
fn lu_without_gpt_and_corrupt_gpts() {
    let (mut m, d) = rig(phone_device());
    up(&mut m, d.clone());
    let mut hc = Ufshc::new(&mut m, BASE, d.clone(), NoDelay, config()).unwrap();
    assert_eq!(hc.read_gpt(3, 4096), Err(Error::Gpt(GptError::Signature)));
    drop(hc);
    m.dev.luns[0].data[4096 + 40] ^= 1; // first_usable inside the CRC-covered header
    let mut hc = Ufshc::new(&mut m, BASE, d.clone(), NoDelay, config()).unwrap();
    assert_eq!(hc.read_gpt(0, 4096), Err(Error::Gpt(GptError::HeaderCrc)));
    drop(hc);
    m.dev.luns[1].data[2 * 4096 + 60] ^= 1; // a partition name byte
    let mut hc = Ufshc::new(&mut m, BASE, d, NoDelay, config()).unwrap();
    assert_eq!(hc.read_gpt(1, 4096), Err(Error::Gpt(GptError::EntriesCrc)));
}
