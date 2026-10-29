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
 if args[0]=='find-identity': print('1) '+('A'*40)+' "Developer ID Application: Fixture ('+os.environ.get('IDENTITY_TEAM','TESTTEAM01')+')"')
 if args==['list-keychains','-d','user']: print('    "/fixture/login.keychain-db"\\n    "/fixture/other keychain.keychain-db"')
elif name=='codesign' and '--entitlements' in args and '-d' in args:
 import plistlib
 team='TESTTEAM01'; app=team+'.dev.jrmd.dbx'
 groups=json.loads(os.environ.get('ENT_GROUPS','null')) if os.environ.get('ENT_GROUPS') else [app]
 sys.stdout.buffer.write(plistlib.dumps({'com.apple.application-identifier':os.environ.get('ENT_APP',app),'keychain-access-groups':groups}))
elif name=='codesign' and '-dv' in args:
 print('TeamIdentifier=TESTTEAM01',file=sys.stderr)
 print('Authority=Developer ID Application: Fixture (TESTTEAM01)',file=sys.stderr)
elif name=='xcrun':
 if args[:2]==['notarytool','history'] and os.environ.get('NOTARY_MISSING'): sys.exit(1)
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
            '#!/usr/bin/env bash\nset -eu\nprintf "build\\n" >> "$CALL_LOG"\nprintf "profile=%s\\n" "${DBX_CARGO_PROFILE-unset}" >> "$CALL_LOG"\nprintf "noprofile=%s\\n" "${DBX_NO_PROVISIONING_PROFILE-unset}" >> "$CALL_LOG"\nmkdir -p "$DBX_APP_DIR/Contents/MacOS"\n')
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
        self.assertIn("security list-keychains -d user -s /fixture/login.keychain-db /fixture/other keychain.keychain-db", calls)
        self.assertFalse(list(self.root.glob("dbx-release.*")))

    def test_release_build_ignores_inherited_local_profile(self):
        self.environment["DBX_CARGO_PROFILE"] = "release-local"
        result = self.release("unsigned")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("profile=release\n", (self.root / "calls.log").read_text())

    def test_release_build_never_uses_the_local_default_profile(self):
        result = self.release("unsigned")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("noprofile=1\n", (self.root / "calls.log").read_text())

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

    def test_profile_secret_is_decoded_and_checked_after_signing(self):
        self.environment["APPLE_PROVISIONING_PROFILE_BASE64"] = "Zml4dHVyZQ=="
        (self.root / "scripts/build-macos-app.sh").write_text(
            '#!/usr/bin/env bash\nset -eu\necho "profile=$(cat "$DBX_PROVISIONING_PROFILE")" >> "$CALL_LOG"\n'
            'mkdir -p "$DBX_APP_DIR/Contents/MacOS"\ncp "$DBX_PROVISIONING_PROFILE" "$DBX_APP_DIR/Contents/embedded.provisionprofile"\n')
        result = self.release()
        self.assertEqual(result.returncode, 0, result.stderr)
        calls = (self.root / "calls.log").read_text()
        self.assertIn("profile=fixture", calls)
        self.assertLess(calls.index("profile=fixture"), calls.index("codesign -d --entitlements :-"))
        self.assertEqual([p.name for p in self.archives()], ["DBX-0.1.0-macos-arm64.zip"])

    def signed_with_profile(self, copy=True, **env):
        self.environment["APPLE_PROVISIONING_PROFILE_BASE64"] = "Zml4dHVyZQ=="
        self.environment.update(env)
        install = 'cp "$DBX_PROVISIONING_PROFILE"' if copy else 'echo different >'
        (self.root / "scripts/build-macos-app.sh").write_text(
            '#!/usr/bin/env bash\nmkdir -p "$DBX_APP_DIR/Contents/MacOS"\n'
            + install + ' "$DBX_APP_DIR/Contents/embedded.provisionprofile"\n')
        result = self.release()
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(self.archives(), [])
        self.assertNotIn("notarytool submit", (self.root / "calls.log").read_text())
        return result

    def test_bad_signed_entitlements_block_the_archive(self):
        cases = {
            "empty group array": ({"ENT_GROUPS": "[]"}, "keychain-access-groups"),
            "wrong first group": ({"ENT_GROUPS": '["TESTTEAM01.other", "TESTTEAM01.dev.jrmd.dbx"]'}, "First signed"),
            "wrong app id": ({"ENT_APP": "TESTTEAM01.other"}, "application-identifier"),
        }
        for name, (env, message) in cases.items():
            with self.subTest(name=name):
                self.assertIn(message, self.signed_with_profile(**env).stderr)
                for key in env:
                    del self.environment[key]
                (self.root / "calls.log").unlink()

    def test_embedded_profile_must_match_validated_profile(self):
        self.assertIn("differs", self.signed_with_profile(copy=False).stderr)

    def test_no_profile_skips_entitlement_check(self):
        self.assertEqual(self.release().returncode, 0)
        self.assertNotIn("--entitlements", (self.root / "calls.log").read_text())

    def test_unsigned_archive_is_explicitly_labelled(self):
        for name in list(self.environment):
            if name.startswith("APPLE_"):
                del self.environment[name]
        result = self.release("unsigned")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual([p.name for p in self.archives()], ["DBX-0.1.0-macos-arm64-unsigned.zip"])
        self.assertNotIn("notarytool", (self.root / "calls.log").read_text())

    def local_environment(self):
        for name in list(self.environment):
            if name.startswith("APPLE_"):
                del self.environment[name]
        self.environment["DBX_KEYCHAIN"] = str(self.root / "personal.keychain-db")
        self.environment["DBX_NOTARY_PROFILE"] = "local-profile"

    def test_local_signing_uses_existing_keychain_without_import_or_mutation(self):
        self.local_environment()
        result = self.release("local")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual([p.name for p in self.archives()], ["DBX-0.1.0-macos-arm64.zip"])
        calls = (self.root / "calls.log").read_text()
        for forbidden in ("create-keychain", "delete-keychain", "security import", "list-keychains", "store-credentials"):
            self.assertNotIn(forbidden, calls)
        self.assertIn("--keychain-profile local-profile --keychain " + self.environment["DBX_KEYCHAIN"], calls)
        self.assertLess(calls.index("notarytool history"), calls.index("\nbuild\n"))

    def test_check_local_does_not_build(self):
        self.local_environment()
        result = self.release("check-local")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertNotIn("\nbuild\n", (self.root / "calls.log").read_text())
        self.assertEqual(self.archives(), [])

    def test_missing_local_credentials_fail_before_compilation(self):
        self.local_environment()
        self.environment["NOTARY_MISSING"] = "1"
        self.assertNotEqual(self.release("local").returncode, 0)
        self.assertNotIn("\nbuild\n", (self.root / "calls.log").read_text())

    def test_local_wrong_team_and_identity_are_rejected(self):
        self.local_environment()
        for name, value in (("APPLE_TEAM_ID", "WRONGTEAM1"), ("DBX_SIGNING_NAME", "-")):
            with self.subTest(name=name):
                self.environment[name] = value
                self.assertNotEqual(self.release("local").returncode, 0)
                self.assertNotIn("\nbuild\n", (self.root / "calls.log").read_text())
                del self.environment[name]

    def test_local_rejected_notarization_cannot_package(self):
        self.local_environment()
        self.environment["NOTARY_STATUS"] = "Invalid"
        self.assertNotEqual(self.release("local").returncode, 0)
        self.assertEqual(self.archives(), [])


if __name__ == "__main__":
    unittest.main()
