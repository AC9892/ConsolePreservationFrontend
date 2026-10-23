"""Synthetic regression checks for provider analysis; no console files required.

Run: python -m unittest discover -s tools -p test_logger_provider.py
Requires the analyzer's optional Capstone dependency, but no guest execution.
"""
import hashlib
import unittest
from types import SimpleNamespace
from unittest.mock import Mock, patch

import analyze_logger_provider as provider


class ProviderAnalysisTests(unittest.TestCase):
    def test_load_and_relro_use_file_offsets(self):
        elf = SimpleNamespace(
            b=b'padding!codeDATA',
            segments=[(1, 5, 8, 0x1000, 0, 4),
                      (0x61000010, 4, 12, 0x2000, 0, 4)],
        )
        self.assertEqual(provider.mapped_bytes(elf, 0x1001, 3), b'ode')
        self.assertEqual(provider.mapped_bytes(elf, 0x2000, 4), b'DATA')

    def test_bss_and_cross_segment_reads_are_refused(self):
        elf = SimpleNamespace(b=b'code', segments=[(1, 5, 0, 0x1000, 0, 4)])
        # The bytes immediately following file-backed data must not be inferred
        # from an eventual zero-filled runtime mapping.
        for address, size in [(0x1004, 1), (0x1003, 2), (0xfff, 1), (0x1000, -1)]:
            with self.subTest(address=address, size=size):
                with self.assertRaises(ValueError):
                    provider.mapped_bytes(elf, address, size)

    def test_nonload_segment_does_not_supply_mapped_evidence(self):
        elf = SimpleNamespace(b=b'data', segments=[(2, 4, 0, 0x1000, 0, 4)])
        with self.assertRaises(ValueError):
            provider.mapped_bytes(elf, 0x1000, 4)

    def test_wrong_hash_is_refused_before_elf_parser(self):
        path = Mock()
        path.stat.return_value.st_size = 16
        path.read_bytes.return_value = b'synthetic input'
        with patch.object(provider, 'Elf') as parser:
            with self.assertRaisesRegex(ValueError, 'Unsupported ShellCore hash'):
                provider.analyze(path)
            parser.assert_not_called()

    def test_changed_second_snapshot_is_refused(self):
        first = b'synthetic first snapshot'
        path = Mock()
        path.stat.return_value.st_size = len(first)
        path.read_bytes.return_value = first
        with patch.object(provider, 'EXPECTED', hashlib.sha256(first).hexdigest()):
            with patch.object(provider, 'Elf', return_value=SimpleNamespace(b=b'changed')):
                with self.assertRaisesRegex(ValueError, 'ShellCore changed while reading'):
                    provider.analyze(path)

    def test_oversize_is_refused_before_read(self):
        path = Mock()
        path.stat.return_value.st_size = 128 * 1024 * 1024 + 1
        with self.assertRaisesRegex(ValueError, 'Input exceeds analysis bound'):
            provider.analyze(path)
        path.read_bytes.assert_not_called()


if __name__ == '__main__':
    unittest.main()
