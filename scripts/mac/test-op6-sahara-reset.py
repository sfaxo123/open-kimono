#!/usr/bin/env python3
"""Fake-device tests for op6-sahara-reset.py (no USB, no phone). Run: python3 <this file>."""
import importlib.util
import os
import struct
import unittest

spec = importlib.util.spec_from_file_location(
    "sahara", os.path.join(os.path.dirname(os.path.abspath(__file__)), "op6-sahara-reset.py"))
sahara = importlib.util.module_from_spec(spec)
spec.loader.exec_module(sahara)


def hello(mode, version=2):
    return struct.pack("<12I", 1, 0x30, version, 1, 0x1000, mode, 0, 0, 0, 0, 0, 0)


class Fake:
    def __init__(self, replies):
        self.replies, self.sent = list(replies), []

    def read(self, timeout_ms):
        return self.replies.pop(0) if self.replies else None

    def write(self, data):
        self.sent.append(data)


class SaharaTests(unittest.TestCase):
    def run_fake(self, replies):
        fake = Fake(replies)
        return fake, sahara.run_reset(fake, lambda m: None)

    def test_memory_debug_32_resets(self):
        fake, res = self.run_fake([hello(2), struct.pack("<4I", 9, 16, 0x146BF000, 0x1000),
                                   struct.pack("<2I", 8, 8)])
        self.assertEqual(fake.sent, [sahara.hello_resp(2), sahara.reset_req()])
        self.assertTrue(res["reset_sent"] and res["reset_resp"])
        self.assertEqual(res["offer"]["table_addr"], 0x146BF000)

    def test_memory_debug_64_resets_without_reset_resp(self):
        fake, res = self.run_fake([hello(2, 3), struct.pack("<2I2Q", 0x10, 24, 0x85E00000, 0x2000)])
        self.assertEqual(fake.sent[-1], sahara.reset_req())
        self.assertTrue(res["reset_sent"])
        self.assertFalse(res["reset_resp"])

    def test_no_offer_still_resets(self):
        fake, res = self.run_fake([hello(2)])
        self.assertEqual(fake.sent, [sahara.hello_resp(2), sahara.reset_req()])

    def test_image_transfer_mode_refused_without_writes(self):
        fake = Fake([hello(0)])
        with self.assertRaises(sahara.SaharaError):
            sahara.run_reset(fake, lambda m: None)
        self.assertEqual(fake.sent, [])

    def test_no_hello_and_no_offer_refused_before_reset(self):
        fake = Fake([None, None])
        with self.assertRaises(sahara.SaharaError):
            sahara.run_reset(fake, lambda m: None)
        self.assertEqual(fake.sent, [sahara.hello_resp(2)])

    def test_no_hello_but_offer_resets(self):
        fake, res = self.run_fake([None, struct.pack("<4I", 9, 16, 0x146BF000, 0x1000)])
        self.assertEqual(fake.sent, [sahara.hello_resp(2), sahara.reset_req()])
        self.assertIsNone(res["hello"])

    def test_unexpected_offer_refused_before_reset(self):
        fake = Fake([hello(2), struct.pack("<5I", 3, 20, 13, 0, 0x50)])  # READ_DATA = image tx
        with self.assertRaises(sahara.SaharaError):
            sahara.run_reset(fake, lambda m: None)
        self.assertEqual(fake.sent, [sahara.hello_resp(2)])

    def test_packet_layouts(self):
        self.assertEqual(len(sahara.hello_resp(2)), 0x30)
        self.assertEqual(struct.unpack("<6I", sahara.hello_resp(2)[:24]), (2, 0x30, 2, 1, 0, 2))
        self.assertEqual(sahara.reset_req(), b"\x07\0\0\0\x08\0\0\0")


if __name__ == "__main__":
    unittest.main(verbosity=2)
