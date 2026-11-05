use rmi4_touch_core::{Contact, Pointer, Tracker};

fn finger(x: u16, y: u16) -> Contact {
    Contact {
        slot: 0,
        x,
        y,
        pressure: 50,
    }
}

#[test]
fn press_hold_move_release() {
    let mut tracker = Tracker::default();
    assert_eq!(
        tracker.update(&[finger(540, 1140)]),
        vec![
            Pointer::Move { x: 540, y: 1140 },
            Pointer::Button { pressed: true }
        ]
    );
    assert!(tracker.update(&[finger(540, 1140)]).is_empty());
    assert_eq!(
        tracker.update(&[finger(541, 1140)]),
        vec![Pointer::Move { x: 541, y: 1140 }]
    );
    assert_eq!(
        tracker.update(&[]),
        vec![Pointer::Button { pressed: false }]
    );
    assert!(tracker.update(&[]).is_empty());
}

#[test]
fn a_new_press_at_the_old_position_moves_first() {
    let mut tracker = Tracker::default();
    tracker.update(&[finger(10, 20)]);
    tracker.update(&[]);
    assert_eq!(
        tracker.update(&[finger(10, 20)]),
        vec![
            Pointer::Move { x: 10, y: 20 },
            Pointer::Button { pressed: true }
        ]
    );
}

#[test]
fn release_only_reports_a_held_button() {
    let mut tracker = Tracker::default();
    assert_eq!(tracker.release(), None);
    tracker.update(&[finger(1, 2)]);
    assert_eq!(tracker.release(), Some(Pointer::Button { pressed: false }));
    assert_eq!(tracker.release(), None);
}
