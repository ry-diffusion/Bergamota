"""crimson() arming: which switch files the image gets (no Mach-O work: linking and signing are stubbed).

  python3 -I -m unittest discover -s tools/renegade/tests -v          (from the repo root)
"""
import os
import sys
import tempfile
import unittest
from unittest import mock

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.abspath(os.path.join(HERE, '..')))
import root_tweaks18 as rt  # noqa: E402


class TestCrimsonArm(unittest.TestCase):
    def run_crimson(self, arm, have_libs=True):
        root, libs = tempfile.mkdtemp(), tempfile.mkdtemp()
        if have_libs:
            for n in rt.CRIMSON_LIBS:
                open(os.path.join(libs, n), 'w').close()
        env = {'RENEGADE_CRIMSON_DIR': libs}
        if arm is not None:
            env['RENEGADE_CRIMSON_ARM'] = arm
        with mock.patch.dict(os.environ, env), mock.patch.object(rt, 'crimson_link', return_value=True), \
                mock.patch.object(rt, 'install_file'), mock.patch('subprocess.run'):
            mercury = rt.crimson(root)
        tmp = os.path.join(root, 'private/var/tmp')
        return mercury, sorted(os.listdir(tmp)) if os.path.isdir(tmp) else []

    def test_default_arms_mercury_only(self):
        self.assertEqual(self.run_crimson(None), (True, ['crimson.app.com.apple.MercuryPoster', 'crimson.arm']))

    def test_all_adds_springboard_and_backboardd(self):
        self.assertEqual(self.run_crimson('all'), (True, ['crimson-bb.on', 'crimson.app.com.apple.MercuryPoster',
                                                          'crimson.app.com.apple.springboard', 'crimson.arm']))

    def test_zero_installs_inert(self):
        self.assertEqual(self.run_crimson('0'), (False, []))

    def test_missing_libs_skip(self):
        with mock.patch.object(rt, 'note_skip'):
            self.assertEqual(self.run_crimson(None, have_libs=False), (False, []))


if __name__ == '__main__':
    unittest.main()
