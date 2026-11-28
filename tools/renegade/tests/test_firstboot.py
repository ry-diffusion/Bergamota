"""root_tweaks18.firstboot(): the first-boot job installs lsplug and registers the apps' embedded extensions
(widgets, controls) after the app registration. No root needed; the job's lsplug step runs under /bin/sh with a fake lsplug.

  python3 -I -m unittest discover -s tools/renegade/tests -v          (from the repo root)
"""
import os
import plistlib
import re
import shutil
import subprocess
import sys
import tempfile
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.abspath(os.path.join(HERE, '..')))
import root_tweaks18 as rt  # noqa: E402


def w(path, data=b'\xcf\xfa\xed\xfe fake', mode=0o755):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, 'wb') as f:
        f.write(data)
    os.chmod(path, mode)


class TestFirstbootLsplug(unittest.TestCase):
    def setUp(self):
        self.d = tempfile.mkdtemp(prefix='renegade-fb-')
        self.root, self.sn = os.path.join(self.d, 'root'), os.path.join(self.d, 'bergamotanet')
        w(os.path.join(self.root, 'bin/sh'))
        w(os.path.join(self.root, 'System/Library/xpc/launchd.plist'), plistlib.dumps({'LaunchDaemons': {}}), 0o644)
        w(os.path.join(self.sn, 'ios18/bergamotauicache'))
        self.saved = rt.BERGAMOTANET_DIRS, rt.SKIPPED[:]
        rt.BERGAMOTANET_DIRS = [self.sn]
        self.chown = os.chown, os.lchown
        rt.OwnerRecorder().install()        # no root: chown is recorded, not applied

    def tearDown(self):
        os.chown, os.lchown = self.chown
        rt.BERGAMOTANET_DIRS, rt.SKIPPED[:] = self.saved
        shutil.rmtree(self.d)

    def script(self):
        rt.firstboot(self.root, appreg=True, jb=False, wallpaper=False)
        return open(os.path.join(self.root, rt.FIRSTBOOT_SH)).read()

    def test_installs_lsplug_and_enables_the_step(self):
        w(os.path.join(self.sn, 'ios18/lsplug'))
        s = self.script()
        self.assertTrue(os.access(os.path.join(self.root, 'usr/local/bin/lsplug'), os.X_OK))
        self.assertIn('\nPLUG=1\n', s)
        self.assertIsNone(re.search(r'@[A-Z0-9]+@', s), 'placeholder left in the script')
        self.assertLess(s.index('APPREG=1'), s.index('PLUG=1'), 'extensions must be registered after their apps')
        subprocess.run(['sh', '-n', os.path.join(self.root, rt.FIRSTBOOT_SH)], check=True)

    def test_without_lsplug_the_step_is_off(self):
        s = self.script()
        self.assertIn('\nPLUG=0\n', s)
        self.assertFalse(os.path.exists(os.path.join(self.root, 'usr/local/bin/lsplug')))
        self.assertIn('app extensions (lsplug)', [f for f, _ in rt.SKIPPED])

    def run_step(self, fails):
        """The job's lsplug step with a fake lsplug that fails `fails` times (sleep stubbed). -> (rc, calls)"""
        w(os.path.join(self.sn, 'ios18/lsplug'))
        s = self.script()
        step = s[s.index('PLUG=1'):s.index('WALL=')]
        count, fake = os.path.join(self.d, 'calls'), os.path.join(self.d, 'lsplug')
        w(fake, ('#!/bin/sh\necho x >>%s\n[ $(wc -l <%s) -gt %d ]\n' % (count, count, fails)).encode())
        step = step.replace('LSP=/usr/local/bin/lsplug', 'LSP=' + fake)
        out = subprocess.run(['sh', '-c', 'sleep() { :; }\nrc=0\n' + step + 'echo rc=$rc\n'],
                             capture_output=True, text=True, check=True).stdout
        return int(out.strip().split('rc=')[-1]), len(open(count).read().split())

    def test_step_retries_until_lsplug_succeeds(self):
        self.assertEqual(self.run_step(3), (0, 4))

    def test_step_gives_up_and_marks_the_run_incomplete(self):
        self.assertEqual(self.run_step(1000), (1, 30))


if __name__ == '__main__':
    unittest.main()
