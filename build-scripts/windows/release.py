#!/usr/bin/env python3
# SPDX-License-Identifier: MPL-2.0
"""Ubuntu-hosted Windows release build, signing and publication (Python 3.11+)."""
import argparse
import base64
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import tempfile
import tomllib
import urllib.error
import urllib.parse
import urllib.request
import uuid
import zipfile

ROOT = Path(__file__).resolve().parents[2]
OUT = ROOT / 'dist/windows/release'
DOCS = ('MANUAL-TESTS.md', 'VALIDATION.md', 'IMPLEMENTATION.md', 'RELEASING.md')
GITHUB_REPO = 'firebadnofire/NightLightAuto'
ALLOWED_LICENSES = {'MIT', 'Unlicense', 'Apache-2.0', 'Unicode-3.0', 'BSD-2-Clause',
                    'BSD-3-Clause', 'ISC', 'Zlib', 'MPL-2.0'}


def run(args, **kwargs):
    return subprocess.run([str(a) for a in args], cwd=ROOT, check=True, **kwargs)


def tag_version(tag):
    match = re.fullmatch(r'[vV](0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)'
                         r'(-[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*)?', tag)
    if not match or any(int(n) > 65535 for n in match.groups()[:3]):
        raise ValueError('Release tag must be vMAJOR.MINOR.PATCH or VMAJOR.MINOR.PATCH '
                         '(optional SemVer prerelease); numeric fields must fit 0..65535')
    suffix = match.group(4)
    if suffix and any(p.isdecimal() and len(p) > 1 and p[0] == '0' for p in suffix[1:].split('.')):
        raise ValueError('Numeric prerelease identifiers must not have leading zeros')
    return tag[1:]


def release_tag():
    tag = os.environ.get('RELEASE_TAG', '')
    tag_version(tag)
    return tag


def prepare_version():
    tag = release_tag()
    sha = run(['git', 'rev-parse', f'refs/tags/{tag}^{{commit}}'], capture_output=True, text=True).stdout.strip()
    head = run(['git', 'rev-parse', 'HEAD'], capture_output=True, text=True).stdout.strip()
    expected = os.environ.get('EXPECTED_SHA', head)
    if sha != head or head != expected:
        raise ValueError('Checked-out commit does not match the triggering tag/SHA')
    version = tag_version(tag)
    manifest = ROOT / 'Cargo.toml'
    content = manifest.read_text(encoding='utf-8')
    content, count = re.subn(r'(?m)^version = "[^"]+"$', f'version = "{version}"', content, count=1)
    if count != 1:
        raise ValueError('Cargo package version field missing')
    manifest.write_text(content, encoding='utf-8')
    lock = ROOT / 'Cargo.lock'
    content = lock.read_text(encoding='utf-8')
    content, count = re.subn(r'(\[\[package\]\]\nname = "fullscreen-night-light"\nversion = ")[^"]+(")',
                             lambda m: m[1] + version + m[2], content, count=1)
    if count != 1:
        raise ValueError('Cargo.lock root package not found')
    lock.write_text(content, encoding='utf-8')
    print(f'Release {tag}: application version {version}, commit {sha}')


def check_version():
    version = tag_version(release_tag())
    manifest = tomllib.loads((ROOT / 'Cargo.toml').read_text(encoding='utf-8'))
    if manifest['package']['version'] != version:
        raise ValueError('Run the version command before building a release')
    return version


def dependency_notices(destination):
    result = run(['cargo', 'metadata', '--locked', '--format-version', '1'], capture_output=True, text=True)
    packages = sorted(json.loads(result.stdout)['packages'], key=lambda p: (p['name'], p['version']))
    text = ['THIRD-PARTY NOTICES - Fullscreen Night Light',
            'Project license: MPL-2.0. Dependencies retain their own licenses.']
    for package in packages:
        if package['name'] == 'fullscreen-night-light':
            continue
        expression = package.get('license') or ''
        identifiers = re.split(r'\s+(?:OR|AND)\s+', expression.replace('(', '').replace(')', ''))
        if not identifiers or any(i not in ALLOWED_LICENSES for i in identifiers):
            raise ValueError(f'Dependency license requires review: {package["name"]} {expression}')
        licenses = sorted(p for p in Path(package['manifest_path']).parent.iterdir()
                          if p.is_file() and re.match(r'^(LICENSE|COPYING|COPYRIGHT|UNLICENSE)', p.name, re.I))
        if not licenses:
            raise ValueError(f'No license text for {package["name"]}')
        text += ['\n' + '=' * 60, f'{package["name"]} {package["version"]} - {expression}',
                 package.get('repository') or '']
        for license_file in licenses:
            text += [f'--- {license_file.name} ---', license_file.read_text(encoding='utf-8')]
    destination.write_text('\n'.join(text), encoding='utf-8')


def payload(executable):
    stage = ROOT / '.build-temp/release-payload'
    stage.mkdir(parents=True, exist_ok=True)
    files = ['FullscreenNightLight.exe', 'README.md', 'LICENSE', 'config.example.toml',
             'THIRD-PARTY-NOTICES.txt'] + [f'docs/{name}' for name in DOCS]
    for name in files:
        dest = stage / name
        dest.parent.mkdir(parents=True, exist_ok=True)
        if name == 'THIRD-PARTY-NOTICES.txt':
            dependency_notices(dest)
        else:
            shutil.copyfile(executable if name.endswith('.exe') else ROOT / name, dest)
    return stage, files


def build():
    version = check_version()
    run(['cargo', 'fmt', '--check'])
    run(['cargo', 'xwin', 'clippy', '--locked', '--all-targets', '--target',
         'x86_64-pc-windows-msvc', '--', '-D', 'warnings'])
    run(['cargo', 'xwin', 'build', '--release', '--locked', '--target', 'x86_64-pc-windows-msvc'])
    OUT.mkdir(parents=True, exist_ok=True)
    stem = f'FullscreenNightLight-{version}-windows-x64'
    target = Path(os.environ.get('CARGO_TARGET_DIR', ROOT / 'target'))
    if not target.is_absolute():
        target = ROOT / target
    executable = target / 'x86_64-pc-windows-msvc/release/FullscreenNightLight.exe'
    stage, files = payload(executable)
    shutil.copyfile(executable, OUT / f'{stem}.exe')
    with zipfile.ZipFile(OUT / f'{stem}-portable.zip', 'w', zipfile.ZIP_DEFLATED) as archive:
        for name in files:
            archive.write(stage / name, f'FullscreenNightLight/{name}')
    run(['makensis', '-V2', f'-DAPP_VERSION={version}',
         f'-DWINDOWS_VERSION={version.split("-")[0]}.0', f'-DPAYLOAD_DIR={stage}',
         f'-DINSTALLER_OUT={OUT / (stem + "-setup.exe")}',
         ROOT / 'build-scripts/windows/installer.nsi'])
    # Source distribution from the exact tag; include the derived Cargo versions.
    result = run(['git', 'archive', '--format=zip', f'refs/tags/{release_tag()}'], capture_output=True)
    import io
    with zipfile.ZipFile(io.BytesIO(result.stdout)) as source, \
            zipfile.ZipFile(OUT / f'NightLightAuto-{version}-source.zip', 'w', zipfile.ZIP_DEFLATED) as archive:
        for name in source.namelist():
            if name.endswith('/'):
                continue
            data = (ROOT / name).read_bytes() if name in ('Cargo.toml', 'Cargo.lock') else source.read(name)
            archive.writestr(f'NightLightAuto-{version}/{name}', data)
    print(f'Built release artifacts in {OUT}')


def release_files():
    version = check_version()
    stem = f'FullscreenNightLight-{version}-windows-x64'
    return [OUT / name for name in (f'{stem}.exe', f'{stem}-portable.zip', f'{stem}-setup.exe',
                                   f'NightLightAuto-{version}-source.zip')]


def sha256(path):
    with path.open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def sign():
    files = release_files()
    checksums = OUT / 'SHA256SUMS'
    checksums.write_text(''.join(f'{sha256(p)}  {p.name}\n' for p in files), encoding='ascii')
    key = os.environ.pop('CI_KEY_B64', '')
    passphrase = os.environ.pop('CI_KEY_PASSPHRASE', None)
    if not key or passphrase is None:
        raise ValueError('CI_KEY_B64 and CI_KEY_PASSPHRASE are required')
    try:
        secret_key = base64.b64decode(''.join(key.split()), validate=True)
    except ValueError:
        raise ValueError('CI_KEY_B64 is not valid base64') from None
    with tempfile.TemporaryDirectory(prefix='nightlight-gpg-') as home:
        os.chmod(home, 0o700)
        gpg = ['gpg', '--homedir', home, '--batch', '--no-tty']
        try:
            run(gpg + ['--import'], input=secret_key, capture_output=True)
            listing = run(gpg + ['--with-colons', '--list-secret-keys'], capture_output=True, text=True).stdout
            fingerprints = []
            primary = False
            for line in listing.splitlines():
                fields = line.split(':')
                if fields[0] == 'sec':
                    primary = True
                elif fields[0] == 'fpr' and primary:
                    fingerprints.append(fields[9])
                    primary = False
            if len(fingerprints) != 1:
                raise ValueError('CI_KEY_B64 must contain exactly one primary private key')
            print(f'OpenPGP signing fingerprint: {fingerprints[0]}')
            for path in files + [checksums]:
                signature = Path(str(path) + '.asc')
                run(gpg + ['--yes', '--pinentry-mode', 'loopback', '--passphrase-fd', '0',
                           '--local-user', fingerprints[0], '--digest-algo', 'SHA256', '--armor',
                           '--detach-sign', '--output', signature, path],
                    input=(passphrase + '\n').encode(), capture_output=True)
                run(gpg + ['--verify', signature, path], capture_output=True)
                print(f'Signed and verified {path.name}.asc')
        finally:
            cleanup = subprocess.run(['gpgconf', '--homedir', home, '--kill', 'gpg-agent'], check=False,
                                     stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
            if cleanup.returncode:
                print('WARNING: unable to stop the temporary GPG agent; its home will still be removed.',
                      file=sys.stderr)


class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, req, fp, code, msg, headers, newurl):
        return None


def https_url(url):
    parsed = urllib.parse.urlsplit(url)
    if parsed.scheme != 'https' or not parsed.netloc or parsed.username or parsed.password:
        raise ValueError('Release endpoints must use HTTPS without URL credentials')
    return url.rstrip('/')


class API:
    def __init__(self, base, token, github=False):
        self.base = https_url(base)
        self.token = token
        self.github = github
        self.opener = urllib.request.build_opener(NoRedirect())

    def request(self, method, path, data=None, content_type='application/json', optional=False):
        url = path if path.startswith('https://') else self.base + path
        # Only API hosts receive credentials. Never forward secrets through redirects.
        allowed = {urllib.parse.urlsplit(self.base).netloc}
        if self.github:
            allowed.add('uploads.github.com')
        if urllib.parse.urlsplit(https_url(url)).netloc not in allowed:
            raise ValueError('Refusing to send release credentials to another host')
        headers = {'Authorization': f'{"Bearer" if self.github else "token"} {self.token}',
                   'User-Agent': 'NightLightAuto-release', 'Content-Type': content_type}
        if self.github:
            headers['Accept'] = 'application/vnd.github+json'
            headers['X-GitHub-Api-Version'] = '2022-11-28'
        request = urllib.request.Request(url, data=data, headers=headers, method=method)
        try:
            with self.opener.open(request, timeout=120) as response:
                raw = response.read()
                return json.loads(raw) if raw else None
        except urllib.error.HTTPError as error:
            if optional and error.code == 404:
                return None
            raise RuntimeError(f'{method} {urllib.parse.urlsplit(url).path}: HTTP {error.code}; '
                               'check token permissions and repository/release state') from None
        except urllib.error.URLError:
            raise RuntimeError('Release API connection failed; check HTTPS connectivity') from None

    def json(self, method, path, value):
        return self.request(method, path, json.dumps(value).encode())

    def upload(self, repo, release_id, file):
        if self.github:
            path = f'https://uploads.github.com/repos/{repo}/releases/{release_id}/assets?name={urllib.parse.quote(file.name)}'
            return self.request('POST', path, file.read_bytes(), 'application/octet-stream')
        boundary = uuid.uuid4().hex
        body = (f'--{boundary}\r\nContent-Disposition: form-data; name="attachment"; '
                f'filename="{file.name}"\r\nContent-Type: application/octet-stream\r\n\r\n').encode()
        body += file.read_bytes() + f'\r\n--{boundary}--\r\n'.encode()
        return self.request('POST', f'/repos/{repo}/releases/{release_id}/assets', body,
                            f'multipart/form-data; boundary={boundary}')


def mirror_tag(tag, token):
    # Scoped git config in environment: no credential in arguments, URL or disk.
    env = os.environ.copy()
    env.update(GIT_TERMINAL_PROMPT='0', GIT_CONFIG_COUNT='1',
               GIT_CONFIG_KEY_0='http.https://github.com/.extraheader',
               GIT_CONFIG_VALUE_0='AUTHORIZATION: basic ' + base64.b64encode(f'x-access-token:{token}'.encode()).decode())
    run(['git', 'push', f'https://github.com/{GITHUB_REPO}.git',
         f'refs/tags/{tag}:refs/tags/{tag}'], env=env, capture_output=True)


def stage_release(api, repo, tag, files, body):
    base = f'/repos/{repo}/releases'
    release = api.request('GET', base + '/tags/' + urllib.parse.quote(tag, safe=''), optional=True)
    if release is None:
        release = api.json('POST', base, dict(tag_name=tag, name=f'Fullscreen Night Light {tag}',
                                             body=body, draft=True, prerelease='-' in tag))
    assets = []
    page = 1
    while True:
        batch = api.request('GET', f'{base}/{release["id"]}/assets?per_page=50&limit=50&page={page}')
        assets.extend(batch)
        if len(batch) < 50:
            break
        page += 1
    existing = {asset['name']: asset for asset in assets}
    for file in files:
        old = existing.get(file.name)
        if old:
            # Published assets are immutable. Reruns may safely replace staged assets.
            if not release['draft']:
                digest = old.get('digest', '')
                if digest == 'sha256:' + sha256(file):
                    continue
                raise ValueError(f'{file.name} already published; refusing to overwrite. '
                                 'Use the existing signed files for a retry or create a new version tag.')
            api.request('DELETE', f'{base}/{release["id"]}/assets/{old["id"]}')
        api.upload(repo, release['id'], file)
    return release


def publish():
    tag = release_tag()
    files = release_files() + [OUT / 'SHA256SUMS']
    files += [Path(str(path) + '.asc') for path in files]
    for path in files:
        if not path.is_file():
            raise ValueError(f'Missing signed release artifact: {path.name}')
    server = https_url(os.environ['FORGEJO_SERVER_URL'])
    repo = os.environ['FORGEJO_REPOSITORY']
    if not re.fullmatch(r'[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+', repo):
        raise ValueError('Invalid Forgejo repository')
    forgejo = API(server + '/api/v1', os.environ['FORGEJO_TOKEN'])
    github = API('https://api.github.com', os.environ['GH_KEY'], github=True)
    mirror_tag(tag, os.environ['GH_KEY'])
    body = (f'Windows 11 x64 native GUI release {tag}.\n\n'
            'Use the setup executable for per-user installation or extract the portable ZIP. '
            'All payloads and SHA256SUMS have detached OpenPGP signatures (.asc). '
            'Signatures are OpenPGP, not Windows Authenticode.\n\n'
            f'Source: {server}/{repo}/src/tag/{tag}\n')
    first = stage_release(forgejo, repo, tag, files, body)
    second = stage_release(github, GITHUB_REPO, tag, files, body)
    for api, repository, release in ((forgejo, repo, first), (github, GITHUB_REPO, second)):
        if release['draft']:
            api.json('PATCH', f'/repos/{repository}/releases/{release["id"]}', {'draft': False})
    print(f'Published {len(files)} identical assets on Forgejo and GitHub for {tag}')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('command', choices=['version', 'build', 'sign', 'publish'])
    args = parser.parse_args()
    try:
        {'version': prepare_version, 'build': build, 'sign': sign, 'publish': publish}[args.command]()
    except subprocess.CalledProcessError as error:
        # Captured output may contain secrets. Never print it.
        print(f'ERROR: release subprocess failed (exit {error.returncode}); '
              'check tool availability, key/passphrase, tag access or build diagnostics.', file=sys.stderr)
        return 1
    except (ValueError, RuntimeError, OSError, KeyError) as error:
        print(f'ERROR: {error}', file=sys.stderr)
        return 1
    return 0


if __name__ == '__main__':
    sys.exit(main())
