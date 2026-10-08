#!/usr/bin/env python3
"""Exercise release failure gates with fixture Mac tools; not real signing proof."""
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest


MOCK = '''#!/usr/bin/env python3
import json,os,pathlib,sys
name=pathlib.Path(sys.argv[0]).name
args=sys.argv[1:]
with open(os.environ['CALL_LOG'],'a') as log: log.write(name+' '+ ' '.join(args)+'\\n')
if name=='uname': print('Darwin' if args==['-s'] else os.environ.get('FIXTURE_ARCH','arm64'))
elif name=='openssl': print('fixture-keychain-password')
elif name=='security':
 if args[0]=='create-keychain': pathlib.Path(args[-1]).touch()
 if args[0]=='find-identity': print('1) ABCDEF "Developer ID Application: Fixture ('+os.environ.get('IDENTITY_TEAM','TESTTEAM01')+')"')
elif name=='codesign' and '-dv' in args:
 print('TeamIdentifier=TESTTEAM01',file=sys.stderr)
 print('Authority=Developer ID Application: Fixture (TESTTEAM01)',file=sys.stderr)
elif name=='xcrun':
 if args[:2]==['notarytool','submit']: print(json.dumps({'status':os.environ.get('NOTARY_STATUS','Accepted')}))
elif name=='ditto': pathlib.Path(args[-1]).write_text('fixture archive')
elif name=='shasum': print('fixture-checksum  '+args[-1])
'''


class ReleaseTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        (self.root / "scripts").mkdir()
        shutil.copy(Path(__file__).with_name("release-macos.sh"), self.root / "scripts")
        (self.root / "Cargo.toml").write_text('[workspace.package]\nversion="0.1.0"\n')
        (self.root / "scripts/build-macos-app.sh").write_text(
            '#!/usr/bin/env bash\nset -eu\nprintf "build\\n" >> "$CALL_LOG"\nmkdir -p "$DBX_APP_DIR/Contents/MacOS"\n')
        self.bin = self.root / "bin"
        self.bin.mkdir()
        for name in ("uname", "openssl", "security", "codesign", "xcrun", "spctl", "ditto", "shasum"):
            path = self.bin / name
            path.write_text(MOCK)
            path.chmod(0o755)
        self.environment = os.environ.copy()
        for name in list(self.environment):
            if name.startswith(("APPLE_", "DBX_")):
                del self.environment[name]
        self.environment.update(
            PATH=str(self.bin) + os.pathsep + os.environ["PATH"],
            CALL_LOG=str(self.root / "calls.log"), RUNNER_TEMP=str(self.root),
            APPLE_CERTIFICATE_BASE64="Zml4dHVyZQ==", APPLE_CERTIFICATE_PASSWORD="fixture",
            APPLE_TEAM_ID="TESTTEAM01", APPLE_ID="fixture@example.test",
            APPLE_APP_SPECIFIC_PASSWORD="fixture-password",
        )

    def release(self, mode="signed"):
        return subprocess.run(["bash", str(self.root / "scripts/release-macos.sh"), mode],
                              env=self.environment, capture_output=True, text=True)

    def archives(self):
        return list((self.root / "target/macos").glob("DBX-*.zip"))

    def test_signed_archive_only_after_acceptance_and_validation(self):
        result = self.release()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual([p.name for p in self.archives()], ["DBX-0.1.0-macos-arm64.zip"])
        calls = (self.root / "calls.log").read_text()
        self.assertLess(calls.index("notarytool submit"), calls.index("stapler staple"))
        self.assertLess(calls.index("stapler validate"), calls.rindex("ditto"))
        self.assertLess(calls.index("spctl --assess"), calls.rindex("ditto"))
        self.assertIn("security delete-keychain", calls)
        self.assertFalse(list(self.root.glob("dbx-release.*")))

    def test_intel_archive_uses_its_own_asset_name(self):
        self.environment["FIXTURE_ARCH"] = "x86_64"
        result = self.release()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual([p.name for p in self.archives()], ["DBX-0.1.0-macos-x86_64.zip"])

    def test_rejected_notarization_produces_no_download(self):
        self.environment["NOTARY_STATUS"] = "Invalid"
        self.assertNotEqual(self.release().returncode, 0)
        self.assertEqual(self.archives(), [])
        self.assertNotIn("stapler staple", (self.root / "calls.log").read_text())

    def test_wrong_team_stops_before_build(self):
        self.environment["IDENTITY_TEAM"] = "WRONGTEAM1"
        self.assertNotEqual(self.release().returncode, 0)
        self.assertNotIn("\nbuild\n", (self.root / "calls.log").read_text())
        self.assertEqual(self.archives(), [])

    def test_unsigned_archive_is_explicitly_labelled(self):
        for name in list(self.environment):
            if name.startswith("APPLE_"):
                del self.environment[name]
        result = self.release("unsigned")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual([p.name for p in self.archives()], ["DBX-0.1.0-macos-arm64-unsigned.zip"])
        self.assertNotIn("notarytool", (self.root / "calls.log").read_text())


if __name__ == "__main__":
    unittest.main()
