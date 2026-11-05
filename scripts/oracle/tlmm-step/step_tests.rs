    use crate::tests::*;
    use crate::shim::*;
    use super::*;
    use std::cell::RefCell;

    fn is_static<T: 'static>(_: &T) {}

    /// The pure scan clears exactly one pin per call and returns an owned result.
    #[test]
    fn one_step_clears_one_and_returns_owned() {
        let mut en = [false; N];
        en.iter_mut().for_each(|e| *e = true);
        let regs = RefCell::new(std::collections::HashMap::from([(status(3), 1u32), (status(7), 1)]));
        let writes = RefCell::new(Vec::new());
        let read = |o: usize| *regs.borrow().get(&o).unwrap_or(&0);
        let write = |o: usize, v: u32| {
            writes.borrow_mut().push((o, v));
            *regs.borrow_mut().entry(o).or_insert(0) &= !v;
        };
        let s = tlmm_pending_step(&en, 0, 40, read, write).unwrap();
        is_static(&s);
        assert_eq!((s.pin, s.virq, s.next), (3, 43, 4));
        assert_eq!(*writes.borrow(), vec![(status(3), 1)]);
        let s2 = tlmm_pending_step(&en, s.next, 40, read, write).unwrap();
        assert_eq!((s2.pin, s2.virq, s2.next), (7, 47, 8));
        assert_eq!(writes.borrow().len(), 2);
        assert!(tlmm_pending_step(&en, s2.next, 40, read, write).is_none());
        assert!(tlmm_pending_step(&en, N, 40, read, write).is_none());
        assert!(tlmm_pending_step(&en, N + 50, 40, read, write).is_none());
        assert_eq!(writes.borrow().len(), 2, "no clear without a pending enabled pin");
    }

    /// Runtime helper `Tlmm::pending_step` on fake MMIO: one clear, owned step.
    #[test]
    fn runtime_pending_step_single_clear() {
        let sc = Sc { enabled: all(), pending: vec![149, 12], ..Sc::default() };
        let en = setup(&sc);
        let t = make(FAKE_BASE, 7, en, false);
        let s = t.pending_step(0).unwrap();
        is_static(&s);
        assert_eq!((s.pin, s.virq, s.next), (12, 19, 13));
        let tr = take_trace();
        assert_eq!(tr.iter().filter(|e| matches!(e, Ev::Write(..))).count(), 1);
        assert_eq!(tr.last(), Some(&Ev::Write(status(12), 1)));
        let s = t.pending_step(s.next).unwrap();
        assert_eq!((s.pin, s.next), (149, 150));
        assert!(t.pending_step(s.next).is_none());
    }
