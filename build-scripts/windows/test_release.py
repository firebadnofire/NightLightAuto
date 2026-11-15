# SPDX-License-Identifier: MPL-2.0
"""Release regression tests: isolated fixtures only, no network or real secrets."""
import base64
import importlib.util
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location('release', Path(__file__).with_name('release.py'))
release = importlib.util.module_from_spec(spec)
spec.loader.exec_module(release)

NSIS = shutil.which('makensis') or shutil.which('makensis.exe')
if not NSIS and os.name == 'nt':
    for variable in ('ProgramFiles(x86)', 'ProgramFiles'):
        candidate = Path(os.environ.get(variable, '')) / 'NSIS/makensis.exe'
        if candidate.is_file():
            NSIS = str(candidate)
            break


class Fixture(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix='nightlight-release-test-')
        self.root = Path(self.temporary.name)
        self.out = self.root / 'dist/windows/release'
        self.out.mkdir(parents=True)
        self.patches = [patch.object(release, 'ROOT', self.root), patch.object(release, 'OUT', self.out),
                        patch.dict(os.environ, {'RELEASE_TAG': 'V1.2.3'}, clear=False)]
        for item in self.patches:
            item.start()
        (self.root / 'Cargo.toml').write_text('[package]\nname = "fullscreen-night-light"\nversion = "0.1.0"\n')
        (self.root / 'Cargo.lock').write_text('version = 4\n\n[[package]]\nname = "fullscreen-night-light"\nversion = "0.1.0"\n')

    def tearDown(self):
        for item in reversed(self.patches):
            item.stop()
        self.temporary.cleanup()

    def versioned(self):
        (self.root / 'Cargo.toml').write_text('[package]\nname = "fullscreen-night-light"\nversion = "1.2.3"\n')

    def test_tags_and_windows_bounds(self):
        for tag, expected in [('v0.1.0', '0.1.0'), ('V12.3.4', '12.3.4'), ('v1.2.3-rc.1', '1.2.3-rc.1')]:
            self.assertEqual(release.tag_version(tag), expected)
        for tag in ['1.2.3', 'v01.2.3', 'v1.2', 'v65536.2.3', 'v1.2.3-01', 'v1.2.3;evil', 'v1.2.3/branch']:
            with self.subTest(tag=tag), self.assertRaises(ValueError):
                release.tag_version(tag)

    def test_version_matches_tag_and_is_repeatable(self):
        release.run(['git', 'init', '-q'])
        release.run(['git', 'add', 'Cargo.toml', 'Cargo.lock'])
        release.run(['git', '-c', 'user.name=Release test', '-c', 'user.email=test@example.invalid',
                     'commit', '-qm', 'fixture'])
        release.run(['git', 'tag', 'V1.2.3'])
        head = release.run(['git', 'rev-parse', 'HEAD'], capture_output=True, text=True).stdout.strip()
        with patch.dict(os.environ, {'EXPECTED_SHA': head}):
            release.prepare_version()
            release.prepare_version()
        self.assertEqual(release.check_version(), '1.2.3')
        lock = release.tomllib.loads((self.root / 'Cargo.lock').read_text())
        self.assertEqual(lock['package'][0]['version'], '1.2.3')
        with patch.dict(os.environ, {'EXPECTED_SHA': '0' * 40}), self.assertRaises(ValueError):
            release.prepare_version()

    def test_wrong_manifest_version_fails(self):
        with self.assertRaises(ValueError):
            release.check_version()

    @unittest.skipUnless(NSIS, 'NSIS compiler is required')
    def test_installer_compiles_with_native_payload_path_and_spaces(self):
        payload = self.root / 'Payload With Spaces'
        names = ['FullscreenNightLight.exe', 'README.md', 'LICENSE', 'config.example.toml',
                 'THIRD-PARTY-NOTICES.txt'] + [f'docs/{name}' for name in release.DOCS]
        for name in names:
            file = payload / name
            file.parent.mkdir(parents=True, exist_ok=True)
            file.write_bytes(b'Isolated installer compilation fixture\n')
        output = self.root / 'Setup With Spaces.exe'
        prefix = '/' if os.name == 'nt' else '-'
        result = subprocess.run([NSIS, f'{prefix}V2', f'{prefix}DAPP_VERSION=1.2.3',
                                 f'{prefix}DWINDOWS_VERSION=1.2.3.0',
                                 f'{prefix}DPAYLOAD_DIR={payload}', f'{prefix}DINSTALLER_OUT={output}',
                                 str(Path(__file__).with_name('installer.nsi').resolve())],
                                capture_output=True, text=True)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertEqual(output.read_bytes()[:2], b'MZ')

    def test_lowercase_license_files_are_packaged(self):
        package = self.root / 'dependency'
        package.mkdir()
        (package / 'license-mit').write_text('test license text')
        metadata = {'packages': [{'name': 'dependency', 'version': '1.0.0', 'license': 'MIT',
                                 'repository': None, 'manifest_path': str(package / 'Cargo.toml')}]}
        with patch.object(release, 'run', return_value=SimpleNamespace(stdout=json.dumps(metadata))):
            destination = self.out / 'notices.txt'
            release.dependency_notices(destination)
        self.assertIn('test license text', destination.read_text())

    def test_unreviewed_license_blocks_packaging(self):
        metadata = {'packages': [{'name': 'dependency', 'version': '1.0.0', 'license': 'Unknown'}]}
        with patch.object(release, 'run', return_value=SimpleNamespace(stdout=json.dumps(metadata))), \
                self.assertRaises(ValueError):
            release.dependency_notices(self.out / 'notices.txt')

    def test_github_tag_push_keeps_secret_out_of_arguments(self):
        with patch.object(release, 'run') as command:
            release.mirror_tag('V1.2.3', 'disposable-token')
        args, kwargs = command.call_args
        self.assertNotIn('disposable-token', str(args))
        self.assertIn('refs/tags/V1.2.3:refs/tags/V1.2.3', args[0])
        self.assertNotIn('--force', args[0])
        self.assertEqual(kwargs['env']['GIT_CONFIG_COUNT'], '1')

    def test_rejects_http_or_url_credentials(self):
        for url in ['http://example.invalid', 'https://secret@example.invalid']:
            with self.assertRaises(ValueError):
                release.https_url(url)

    def test_credentials_never_sent_to_foreign_host(self):
        api = release.API('https://example.invalid/api/v1', 'test-not-a-real-token')
        with self.assertRaises(ValueError):
            api.request('POST', 'https://other.invalid/upload', b'test')

    def test_new_release_is_draft_with_exact_upload_set(self):
        artifact = self.out / 'app.exe'
        artifact.write_bytes(b'fixture')
        api = FakeAPI()
        result = release.stage_release(api, 'owner/repo', 'v1.2.3', [artifact], 'notes')
        self.assertTrue(result['draft'])
        self.assertEqual(api.uploads, ['app.exe'])
        self.assertTrue(api.creates[0]['draft'])

    def test_draft_retry_replaces_only_matching_names(self):
        artifact = self.out / 'app.exe'
        artifact.write_bytes(b'fixture')
        api = FakeAPI(existing={'id': 1, 'draft': True}, assets=[{'id': 7, 'name': 'app.exe'}])
        release.stage_release(api, 'owner/repo', 'v1.2.3', [artifact], 'notes')
        self.assertEqual(api.deletes, ['/repos/owner/repo/releases/1/assets/7'])
        self.assertEqual(api.uploads, ['app.exe'])

    def test_published_asset_is_never_replaced(self):
        artifact = self.out / 'app.exe'
        artifact.write_bytes(b'fixture')
        api = FakeAPI(existing={'id': 1, 'draft': False}, assets=[{'id': 7, 'name': 'app.exe'}])
        with self.assertRaises(ValueError):
            release.stage_release(api, 'owner/repo', 'v1.2.3', [artifact], 'notes')
        self.assertFalse(api.deletes)
        self.assertFalse(api.uploads)
        api.assets[0]['digest'] = 'sha256:' + release.sha256(artifact)
        release.stage_release(api, 'owner/repo', 'v1.2.3', [artifact], 'notes')
        self.assertFalse(api.uploads)

    def test_publish_requires_complete_signature_set_before_any_network(self):
        self.versioned()
        with patch.object(release, 'mirror_tag') as mirror, self.assertRaises(ValueError):
            release.publish()
        mirror.assert_not_called()

    @unittest.skipUnless(shutil.which('gpg') and shutil.which('gpgconf'), 'GnuPG is required')
    def test_real_signatures_checksums_and_tamper_rejection(self):
        self.versioned()
        for artifact in release.release_files():
            artifact.write_bytes(b'test payload ' + artifact.name.encode())
        key_home = self.root / 'test-key'
        key_home.mkdir(mode=0o700)
        gpg = ['gpg', '--homedir', str(key_home), '--batch', '--no-tty']
        try:
            subprocess.run(gpg + ['--pinentry-mode', 'loopback', '--passphrase', 'test-only',
                                 '--quick-generate-key', 'Release Test <test@example.invalid>',
                                 'rsa2048', 'sign', '1d'], check=True, capture_output=True)
            exported = subprocess.run(gpg + ['--pinentry-mode', 'loopback', '--passphrase', 'test-only',
                                            '--export-secret-keys'], check=True, capture_output=True).stdout
            with patch.dict(os.environ, {'CI_KEY_B64': base64.b64encode(exported).decode(),
                                         'CI_KEY_PASSPHRASE': 'test-only'}):
                release.sign()
            files = release.release_files() + [self.out / 'SHA256SUMS']
            for file in files:
                subprocess.run(gpg + ['--verify', str(file) + '.asc', str(file)], check=True, capture_output=True)
            lines = (self.out / 'SHA256SUMS').read_text().splitlines()
            self.assertEqual(len(lines), 4)
            for line in lines:
                digest, name = line.split('  ')
                self.assertEqual(digest, release.sha256(self.out / name))
            files[0].write_bytes(b'tampered')
            result = subprocess.run(gpg + ['--verify', str(files[0]) + '.asc', str(files[0])], capture_output=True)
            self.assertNotEqual(result.returncode, 0)
        finally:
            subprocess.run(['gpgconf', '--homedir', str(key_home), '--kill', 'gpg-agent'], check=False,
                           stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)


class FakeAPI:
    def __init__(self, existing=None, assets=None):
        self.existing = existing
        self.assets = assets or []
        self.uploads, self.deletes, self.creates = [], [], []

    def request(self, method, path, **kwargs):
        if '/tags/' in path:
            return self.existing
        if method == 'DELETE':
            self.deletes.append(path)
            return None
        return self.assets

    def json(self, method, path, value):
        self.creates.append(value)
        return {'id': 1, 'draft': True}

    def upload(self, repo, release_id, file):
        self.uploads.append(file.name)


if __name__ == '__main__':
    unittest.main()
