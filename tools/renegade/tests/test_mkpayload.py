"""mkpayload.py cmdline split (patch 1145): CommandLine <= 607 bytes, the rest in /chosen bergamota-bootargs-extra.

  python3 -I -m unittest discover -s tools/renegade/tests -v          (from the repo root)
"""
import os
import sys
import unittest

TOOLS = os.path.abspath(os.path.join(os.path.dirname(os.path.abspath(__file__)), '..', '..'))
sys.path.insert(0, TOOLS)
import adt  # noqa: E402
import mkpayload as mp  # noqa: E402


def tiny_adt():
    root, chosen = adt.Node(), adt.Node()
    root.props = [('name', 0, b'device-tree\0')]
    chosen.props = [('name', 0, b'chosen\0')]
    root.children = [chosen]
    return adt.serialize(root)


class SplitTest(unittest.TestCase):
    def test_short_unchanged(self):
        self.assertEqual(mp.split_cmdline('debug=0xc serial=3'), ('debug=0xc serial=3', ''))

    def test_exactly_607(self):
        cl = 'a' * 607
        self.assertEqual(mp.split_cmdline(cl), (cl, ''))

    def test_split_at_space(self):
        words = ['arg%03d=1' % i for i in range(120)]          # 7 bytes + space each
        cl = ' '.join(words)
        head, extra = mp.split_cmdline(cl)
        self.assertLessEqual(len(head), 607)
        self.assertEqual(head + ' ' + extra, cl)                # nothing lost, no word cut
        self.assertTrue(extra.startswith('arg'))

    def test_refuses_no_space(self):
        with self.assertRaises(SystemExit):
            mp.split_cmdline('a' * 700)

    def test_refuses_huge_extra(self):
        with self.assertRaises(SystemExit):
            mp.split_cmdline('x=1 ' * 500)                      # 2000 bytes: extra > 1024

    def test_adt_property(self):
        blob = mp.add_bootargs_extra(tiny_adt(), 'bergamotasensors=3 bergamotaslpi=2')
        root, _ = adt.parse(blob)
        chosen = next(c for c in root.children if c.name == 'chosen')
        self.assertEqual(chosen.prop('bergamota-bootargs-extra'), b'bergamotasensors=3 bergamotaslpi=2\0')


if __name__ == '__main__':
    unittest.main()
