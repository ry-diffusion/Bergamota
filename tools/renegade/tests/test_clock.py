"""root_tweaks18.bergamotaclock(): binary installed, the two launchd jobs (restore at load, save every 600 s), skip when not built.

  python3 -I -m unittest discover -s tools/renegade/tests -v          (from the repo root)
"""
import os
import plistlib
import shutil
import subprocess
import sys
import tempfile
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.abspath(os.path.join(HERE, '..')))
import root_tweaks18 as rt  # noqa: E402

LP = 'System/Library/xpc/launchd.plist'


class TestClock(unittest.TestCase):
    def setUp(self):
        self.d = tempfile.mkdtemp(prefix='renegade-clock-')
        self.root, self.sn = os.path.join(self.d, 'root'), os.path.join(self.d, 'bergamotanet')
        os.makedirs(os.path.dirname(os.path.join(self.root, LP)))
        with open(os.path.join(self.root, LP), 'wb') as f:
            plistlib.dump({'LaunchDaemons': {'/x.plist': {'Label': 'x'}}}, f)
        self.saved = rt.BERGAMOTANET_DIRS, rt.SKIPPED[:]
        rt.BERGAMOTANET_DIRS = [self.sn]
        self.chown = os.chown, os.lchown
        rt.OwnerRecorder().install()

    def tearDown(self):
        os.chown, os.lchown = self.chown
        rt.BERGAMOTANET_DIRS, rt.SKIPPED[:] = self.saved
        shutil.rmtree(self.d)

    def build(self):
        p = os.path.join(self.sn, 'ios18/bergamotaclock')
        os.makedirs(os.path.dirname(p))
        open(p, 'wb').write(b'\xcf\xfa\xed\xfe fake')

    def jobs(self):
        with open(os.path.join(self.root, LP), 'rb') as f:
            return plistlib.load(f)['LaunchDaemons']

    def test_jobs_and_binary(self):
        self.build()
        rt.bergamotaclock(self.root)
        self.assertTrue(os.access(os.path.join(self.root, 'usr/local/bin/bergamotaclock'), os.X_OK))
        j = self.jobs()
        self.assertIn('/x.plist', j)
        r = j['/System/Library/LaunchDaemons/org.bergamota.clock-restore.plist']
        s = j['/System/Library/LaunchDaemons/org.bergamota.clock-save.plist']
        self.assertEqual(r['ProgramArguments'], ['/usr/local/bin/bergamotaclock', 'restore'])
        self.assertEqual(s['ProgramArguments'], ['/usr/local/bin/bergamotaclock', 'save'])
        self.assertIs(r['RunAtLoad'], True)
        self.assertNotIn('StartInterval', r)
        self.assertIs(s['RunAtLoad'], False)
        self.assertEqual(s['StartInterval'], 600)
        self.assertEqual((r['UserName'], s['UserName']), ('root', 'root'))
        for label in ('restore', 'save'):
            self.assertTrue(os.path.exists(os.path.join(self.root, 'System/Library/LaunchDaemons/org.bergamota.clock-%s.plist' % label)))

    def test_idempotent(self):
        self.build()
        rt.bergamotaclock(self.root)
        rt.bergamotaclock(self.root)
        self.assertEqual(len([k for k in self.jobs() if 'clock' in k]), 2)

    def test_not_built_is_skipped(self):
        rt.bergamotaclock(self.root)
        self.assertEqual(list(self.jobs()), ['/x.plist'])
        self.assertTrue(any('bergamotaclock' in f for f, _ in rt.SKIPPED))

    def test_live_plist_mode(self):
        src, out = os.path.join(self.root, LP), os.path.join(self.d, 'out.plist')
        subprocess.run([sys.executable, '-I', os.path.join(HERE, '..', 'root_tweaks18.py'), '--clock-plist', src, out], check=True, capture_output=True)
        with open(out, 'rb') as f:
            j = plistlib.load(f)['LaunchDaemons']
        self.assertEqual(sorted(j), sorted(['/x.plist'] + ['/System/Library/LaunchDaemons/org.bergamota.clock-%s.plist' % k for k in ('restore', 'save')]))


if __name__ == '__main__':
    unittest.main()
