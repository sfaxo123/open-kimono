use rmi4_touch_core::{Attention, Error};

/// `interrupts-extended = <0x4a 0x7d 0x02>` from the OnePlus 6 U-Boot DTB:
/// TLMM phandle 0x4a, GPIO 125, IRQ_TYPE_EDGE_FALLING.
const ENCHILADA: [u8; 12] = [0, 0, 0, 0x4a, 0, 0, 0, 0x7d, 0, 0, 0, 2];

#[test]
fn parses_the_enchilada_attention_specifier() {
    let attention = Attention::parse(&ENCHILADA).unwrap();
    assert_eq!(
        attention,
        Attention {
            controller: 0x4a,
            pin: 125,
            flags: 2
        }
    );
    assert_eq!(attention.irq_path(), "/scheme/irq/phandle-74/125,2");
}

#[test]
fn rejects_malformed_specifiers() {
    assert_eq!(
        Attention::parse(&ENCHILADA[..8]),
        Err(Error::InvalidAttention)
    );
    let mut two = ENCHILADA.to_vec();
    two.extend_from_slice(&ENCHILADA);
    assert_eq!(Attention::parse(&two), Err(Error::InvalidAttention));
    let mut no_phandle = ENCHILADA;
    no_phandle[3] = 0;
    assert_eq!(Attention::parse(&no_phandle), Err(Error::InvalidAttention));
    let mut no_trigger = ENCHILADA;
    no_trigger[11] = 0;
    assert_eq!(Attention::parse(&no_trigger), Err(Error::InvalidAttention));
}
