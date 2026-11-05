//! Happy-path bring-up against the register + UPIU model (`specs/ufs.md` §4).

mod common;

use common::device::Seen;
use common::model::CORE_CLK_CTRL_RESET;
use common::{config, phone_device, rig, BASE};
use mmio_replay::{Op, Recorder};
use ufs_core::regs::{self, qcom};
use ufs_core::scsi::Cmd;
use ufs_core::{LogicalUnit, NoDelay, Ufshc};

#[test]
fn bring_up_enumerates_enabled_logical_units() {
    let (mut m, dma) = rig(phone_device());
    let info = Ufshc::new(&mut m, BASE, dma, NoDelay, config())
        .unwrap()
        .bring_up()
        .unwrap();
    assert_eq!(info.host.cap, 0x0107_001f);
    assert_eq!(info.host.qcom_hw_version, 0x3001_0000);
    assert_eq!(
        (
            info.spec_version,
            info.manufacturer_id,
            info.number_lu,
            info.max_lu
        ),
        (0x0210, 0x01ce, 3, 8)
    );
    assert_eq!(info.raw_capacity, 0x0773_0000);
    let lu = |lun, block_count, boot_lun_id, write_protect| LogicalUnit {
        lun,
        block_size: 4096,
        block_count,
        write_protect,
        boot_lun_id,
    };
    assert_eq!(
        info.units,
        vec![lu(0, 64, 0, 0), lu(1, 16, 1, 1), lu(3, 8, 0, 0)]
    );
    // Unit descriptors for all 8 LUNs were read, nothing beyond.
    let unit_reads = m
        .dev
        .seen
        .iter()
        .filter(|s| {
            matches!(
                s,
                Seen::Query {
                    opcode: 1,
                    idn: 2,
                    ..
                }
            )
        })
        .count();
    assert_eq!(unit_reads, 8);
}

#[test]
fn qualcomm_vendor_setup_is_applied_before_link_startup() {
    let (mut m, dma) = rig(phone_device());
    Ufshc::new(&mut m, BASE, dma, NoDelay, config())
        .unwrap()
        .bring_up()
        .unwrap();
    assert_eq!(m.cfg1 & qcom::CFG1_QUNIPRO_SEL, 1);
    assert_eq!(m.cfg0, 0, "CFG0 untouched on HW major 3");
    assert_eq!(m.ie, 0, "polling: interrupts stay disabled");
    let clk = m.dme[&(qcom::DME_VS_CORE_CLK_CTRL, 0)];
    assert_ne!(clk, CORE_CLK_CTRL_RESET);
    assert_eq!(clk & 0x1ff, 150, "150 cycles/us, CORE_CLK_DIV_EN cleared");
    assert_eq!(m.dme[&(regs::TX_LCC_ENABLE, 0)], 0);
    assert_eq!(m.dme[&(regs::TX_LCC_ENABLE, 1)], 0);
    let ops: Vec<u8> = m.uic_log.iter().map(|u| u.0).collect();
    // HIBERN8 check, core clk RMW, lane count, 2 × LCC off, link startup.
    assert_eq!(ops, vec![1, 1, 2, 1, 2, 2, 0x16]);
    assert!(m.link_up && m.rsr == 1);
    assert_eq!(m.utrlba, common::DMA_PHYS);
}

#[test]
fn enabled_controller_is_cycled_through_hce_zero() {
    let (mut m, dma) = rig(phone_device());
    m.hce = true; // U-Boot left it running
    let mut hc = Ufshc::new(Recorder::new(&mut m), BASE, dma, NoDelay, config()).unwrap();
    hc.enable().unwrap();
    let trace = hc.into_parts().0.into_trace();
    let hce: Vec<u64> = trace
        .events
        .iter()
        .filter(|e| e.op == Op::Write && e.addr == BASE + regs::HCE)
        .map(|e| e.value)
        .collect();
    assert_eq!(hce, vec![0, 1]);
}

#[test]
fn non_qualcomm_config_never_touches_vendor_registers() {
    let (mut m, dma) = rig(phone_device());
    let mut cfg = config();
    cfg.qcom = None;
    let mut hc = Ufshc::new(Recorder::new(&mut m), BASE, dma, NoDelay, cfg).unwrap();
    hc.bring_up().unwrap();
    let trace = hc.into_parts().0.into_trace();
    assert!(
        trace.events.iter().all(|e| e.addr - BASE < 0xc0),
        "vendor window 0xc0.. untouched"
    );
}

#[test]
fn read_capacity_inquiry_and_blocks() {
    let (mut m, dma) = rig(phone_device());
    let mut hc = Ufshc::new(&mut m, BASE, dma, NoDelay, config()).unwrap();
    hc.bring_up().unwrap();
    let cap = hc.read_capacity(0).unwrap();
    assert_eq!((cap.last_lba, cap.block_size), (63, 4096));
    let mut inq = [0u8; 36];
    assert_eq!(hc.scsi(0, Cmd::Inquiry, 0, &mut inq).unwrap(), 36);
    assert_eq!(&inq[8..16], b"SAMSUNG ");
    assert_eq!(hc.scsi(3, Cmd::TestUnitReady, 0, &mut []).unwrap(), 0);
    let mut sense = [0u8; 18];
    assert_eq!(hc.scsi(3, Cmd::RequestSense, 0, &mut sense).unwrap(), 18);
    let mut blk = vec![0u8; 4096];
    hc.read(3, 1, 4096, &mut blk).unwrap();
    assert_eq!(blk[0], 0x42);
}

#[test]
fn large_read_is_chunked_to_the_dma_buffer() {
    let dev = phone_device();
    let expect = dev.luns[0].data[20 * 4096..27 * 4096].to_vec();
    // 3 data blocks of room → a 7-block read needs 3 READ(10)s.
    let dma = common::SharedDma::new(common::DMA_PHYS, 0x1000 + 3 * 4096);
    let mut m = common::model::UfsModel::sdm845(dma.clone(), dev);
    let mut hc = Ufshc::new(&mut m, BASE, dma, NoDelay, config()).unwrap();
    hc.bring_up().unwrap();
    let mut buf = vec![0u8; 7 * 4096];
    hc.read(0, 20, 4096, &mut buf).unwrap();
    assert_eq!(buf, expect);
    drop(hc);
    let reads: Vec<_> = m
        .dev
        .seen
        .iter()
        .filter(|s| matches!(s, Seen::Command { opcode: 0x28, .. }))
        .collect();
    assert_eq!(reads.len(), 3);
}

#[test]
fn gpt_of_user_and_boot_lus() {
    let (mut m, dma) = rig(phone_device());
    let mut hc = Ufshc::new(&mut m, BASE, dma, NoDelay, config()).unwrap();
    hc.bring_up().unwrap();
    let g = hc.read_gpt(0, 4096).unwrap();
    let names: Vec<_> = g
        .entries
        .iter()
        .map(|e| (e.name.as_str(), e.first_lba, e.last_lba))
        .collect();
    assert_eq!(
        names,
        vec![("ssd", 6, 7), ("misc", 8, 9), ("userdata", 10, 58)]
    );
    assert_eq!(
        (
            g.header.current_lba,
            g.header.backup_lba,
            g.header.num_entries
        ),
        (1, 63, 128)
    );
    let b = hc.read_gpt(1, 4096).unwrap();
    assert_eq!(b.entries[1].name, "xbl_config_a");
    assert_eq!(b.entries[0].type_guid, common::disk::TYPE_GUID);
}

#[test]
fn whole_session_sends_no_write_of_any_kind() {
    let (mut m, dma) = rig(phone_device());
    let mut hc = Ufshc::new(&mut m, BASE, dma, NoDelay, config()).unwrap();
    hc.bring_up().unwrap();
    hc.read_gpt(0, 4096).unwrap();
    hc.read_gpt(1, 4096).unwrap();
    hc.read_capacity(3).unwrap();
    assert_eq!(hc.read_attribute(0x00, 0).unwrap(), 1, "bBootLunEn");
    drop(hc);
    assert!(!m.dev.saw_write());
    let flag_writes = m
        .dev
        .seen
        .iter()
        .filter(|s| matches!(s, Seen::Query { function: 0x81, .. }))
        .count();
    assert_eq!(flag_writes, 1, "only SET_FLAG fDeviceInit");
}

#[test]
fn unit_attention_is_retried_once() {
    let mut dev = phone_device();
    dev.faults.unit_attention = true;
    let (mut m, dma) = rig(dev);
    let mut hc = Ufshc::new(&mut m, BASE, dma, NoDelay, config()).unwrap();
    hc.bring_up().unwrap();
    assert_eq!(hc.read_capacity(0).unwrap().last_lba, 63);
}

#[test]
fn transient_nop_failures_are_retried() {
    let mut dev = phone_device();
    dev.faults.nop_ocs = vec![0x05, 0x05];
    dev.faults.device_init_busy = 3;
    let (mut m, dma) = rig(dev);
    Ufshc::new(&mut m, BASE, dma, NoDelay, config())
        .unwrap()
        .bring_up()
        .unwrap();
    assert_eq!(m.dev.seen.iter().filter(|s| **s == Seen::Nop).count(), 3);
    let polls = m
        .dev
        .seen
        .iter()
        .filter(|s| matches!(s, Seen::Query { opcode: 5, .. }))
        .count();
    assert_eq!(polls, 4, "fDeviceInit read until clear");
}
