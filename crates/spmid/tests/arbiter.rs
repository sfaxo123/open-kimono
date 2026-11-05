use mmio_replay::{Event, Replayer, Trace, Width};
use spmid_core::{Arbiter, Button, Buttons, Edge, Error, Levels, Windows};

const CORE: u64 = 0x0c44_0000;
const OBS: u64 = 0x0e60_0000;
const CFG: u64 = 0x0c40_a000;
fn win() -> Windows {
    Windows {
        core: CORE,
        observer: OBS,
        config: CFG,
        owner: 0,
    }
}
fn r(t: &mut Trace, addr: u64, value: u32) {
    t.push(Event::read(addr, Width::W32, value as u64));
}
fn w(t: &mut Trace, addr: u64, value: u32) {
    t.push(Event::write(addr, Width::W32, value as u64));
}
fn setup(entries: &[(u8, u8)]) -> Trace {
    let mut t = Trace::new();
    r(&mut t, CORE, 0x5000_0000);
    r(&mut t, CORE + 4, entries.len() as u32);
    for (i, (sid, pid)) in entries.iter().enumerate() {
        r(
            &mut t,
            CORE + 0x900 + (i * 4) as u64,
            ((*sid as u32) << 16) | ((*pid as u32) << 8),
        );
        r(&mut t, CFG + 0x700 + (i * 4) as u64, 0);
    }
    t
}
fn read(t: &mut Trace, channel: u64, offset: u8, value: u8) {
    let base = OBS + 0x80 * channel;
    w(t, base + 4, 0);
    w(t, base, (1 << 27) | ((offset as u32) << 4) | 1);
    r(t, base + 8, 1);
    r(t, base + 0x18, value as u32);
}
#[test]
fn replay_revision_and_three_buttons() {
    let mut t = setup(&[(0, 1), (0, 8), (0, 0xc4), (0, 0xc5)]);
    read(&mut t, 0, 4, 0x51);
    read(&mut t, 1, 0x10, 0x80);
    read(&mut t, 2, 0x10, 0);
    read(&mut t, 3, 0x10, 1);
    let mut arb = Arbiter::new(Replayer::strict(t), win()).unwrap();
    assert_eq!(arb.pmic_type(), Ok(0x51));
    assert_eq!(
        arb.sample(),
        Ok(Levels {
            power: true,
            volume_down: true,
            volume_up: false
        })
    );
    assert!(arb.into_io().finish().ok());
}
#[test]
fn missing_channel_fails_without_bus_access() {
    let t = setup(&[(0, 8)]);
    let mut arb = Arbiter::new(Replayer::strict(t), win()).unwrap();
    assert_eq!(arb.read_byte(0, 0xc410), Err(Error::BadChannel));
    assert!(arb.into_io().finish().ok());
}
#[test]
fn timeout_is_bounded() {
    let mut t = setup(&[(0, 8)]);
    w(&mut t, OBS + 4, 0);
    w(&mut t, OBS, (1 << 27) | (0x10 << 4) | 1);
    for _ in 0..1024 {
        r(&mut t, OBS + 8, 0);
    }
    let mut arb = Arbiter::new(Replayer::strict(t), win()).unwrap();
    assert_eq!(arb.read_byte(0, 0x810), Err(Error::Timeout));
    assert!(arb.into_io().finish().ok());
}
#[test]
fn bad_status_is_rejected() {
    let mut t = setup(&[(0, 8)]);
    w(&mut t, OBS + 4, 0);
    w(&mut t, OBS, (1 << 27) | (0x10 << 4) | 1);
    r(&mut t, OBS + 8, 3);
    let mut arb = Arbiter::new(Replayer::strict(t), win()).unwrap();
    assert_eq!(arb.read_byte(0, 0x810), Err(Error::BadStatus(3)));
    assert!(arb.into_io().finish().ok());
}
#[test]
fn unsupported_version_is_rejected() {
    let mut t = Trace::new();
    r(&mut t, CORE, 0x3000_0000);
    let result = Arbiter::new(Replayer::strict(t), win());
    assert!(matches!(
        result,
        Err(Error::UnsupportedVersion(0x3000_0000))
    ));
}
#[test]
fn edges_have_no_initial_press_and_power_is_gated() {
    let mut buttons = Buttons::new(false);
    let released = Levels {
        power: false,
        volume_down: false,
        volume_up: false,
    };
    assert!(buttons.update(released).is_empty());
    assert_eq!(
        buttons.update(Levels {
            power: true,
            volume_down: true,
            volume_up: false
        }),
        vec![Edge {
            button: Button::VolumeDown,
            pressed: true
        }]
    );
    assert_eq!(
        buttons.update(Levels {
            power: false,
            volume_down: false,
            volume_up: false
        }),
        vec![Edge {
            button: Button::VolumeDown,
            pressed: false
        }]
    );
}

#[test]
fn confirmed_power_edges_reach_input_model() {
    let mut buttons = Buttons::new(true);
    let released = Levels {
        power: false,
        volume_down: false,
        volume_up: false,
    };
    assert!(buttons.update(released).is_empty());
    assert_eq!(
        buttons.update(Levels {
            power: true,
            ..released
        }),
        vec![Edge {
            button: Button::Power,
            pressed: true
        }]
    );
    assert_eq!(
        buttons.update(released),
        vec![Edge {
            button: Button::Power,
            pressed: false
        }]
    );
}

#[test]
fn sparse_v5_map_uses_real_apids_and_rejects_missing_gpio() {
    let mut t = Trace::new();
    r(&mut t, CORE, 0x5000_0000);
    r(&mut t, CORE + 4, 512);
    for apid in 0..512 {
        let entry = match apid {
            1 => 0x0100,
            34 => 0x0800,
            _ => 0,
        };
        r(&mut t, CORE + 0x900 + apid * 4, entry);
        if entry != 0 {
            r(&mut t, CFG + 0x700 + apid * 4, 0);
        }
    }
    read(&mut t, 1, 4, 0x51);
    read(&mut t, 34, 0x10, 0);
    let mut arb = Arbiter::new(Replayer::strict(t), win()).unwrap();
    assert_eq!(arb.pmic_type(), Ok(0x51));
    assert_eq!(arb.read_byte(0, 0x810), Ok(0));
    assert_eq!(arb.read_byte(0, 0xc410), Err(Error::BadChannel));
    assert!(arb.into_io().finish().ok());
}
