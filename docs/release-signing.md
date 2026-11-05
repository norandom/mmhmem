# Release signing (owner steps)

Releases are verified by `mmhmem setup` before anything is installed. Two
signatures, made with the same Ed25519 key, protect them:

| What | Signature |
| --- | --- |
| each archive `mmhmem-<tag>-<target>.zip` / `.tar.gz` | zipsign signature embedded in the archive, context = the published file name |
| `SHA256SUMS` (computed after signing, over all assets) | detached `SHA256SUMS.sig`, zipsign format (`zipsign sign separate`), context `SHA256SUMS` |

`SHA256SUMS.sig` is a zipsign signature block: magic header, signature count,
then 64-byte Ed25519ph signatures. Rust reads it with
`zipsign_api::verify::read_signatures`, `Prehash::calculate` and
`verify::find_match` (see `src/setup/release_keys.rs`). Ed25519 is
deterministic, so a retried publish produces identical bytes.

The private key lives only in the GitHub `release` environment. The public key
list is compiled into the binary (`RELEASE_PUBLIC_KEYS` in
`src/setup/release_keys.rs`). The list is empty until you add your key, and
setup refuses to update while it is empty. The publisher also refuses to
publish with a key that is not in that list.

The pinned signing tool is `zipsign` 0.2.2. Never paste a real key into an
issue, a commit, a chat or a log.

## 1. Generate the key pair (once, on your own machine)

```powershell
cargo install zipsign --version =0.2.2 --locked
zipsign --version        # must print: zipsign 0.2.2
New-Item -ItemType Directory -Force $HOME\mmhmem-release-key | Out-Null
zipsign gen-key $HOME\mmhmem-release-key\release.key $HOME\mmhmem-release-key\release.pub
```

`release.key` is the 64-byte private key file. `release.pub` is the 32-byte
public key. Keep `release.key` off shared drives; back it up in a password
manager or offline.

## 2. Store the private key as the `release` environment secret

Create the environment with you as the required reviewer (gh CLI; the commands
look up your numeric user id with `gh api user --jq .id`):

```powershell
$repo = 'norandom/mmhmem'
$reviewer = (gh api user --jq .id)
'{"reviewers":[{"type":"User","id":' + $reviewer + '}],"deployment_branch_policy":null}' |
  gh api --method PUT "repos/$repo/environments/release" --input -
```

Store the key (the value is read from the file and never printed):

```powershell
[Convert]::ToBase64String([IO.File]::ReadAllBytes("$HOME\mmhmem-release-key\release.key")) |
  gh secret set MMHMEM_RELEASE_SIGNING_KEY --env release --repo $repo
```

UI path: Repository, Settings, Environments, New environment `release`, tick
Required reviewers and add yourself (and a second maintainer if you can), then
Environment secrets, Add secret `MMHMEM_RELEASE_SIGNING_KEY` with the base64
text of `release.key`. Do not add this secret as a repository or organization
secret: only the `release` environment, behind approval, may hold it.

Verify (names only, values are never shown):

```powershell
gh secret list --env release --repo $repo
```

## 3. Embed the public key

Print the Rust literal:

```powershell
$b = [IO.File]::ReadAllBytes("$HOME\mmhmem-release-key\release.pub")
if ($b.Length -ne 32) { throw 'unexpected public key size' }
'    [' + (($b | ForEach-Object { '0x' + $_.ToString('x2') }) -join ', ') + '],'
```

Paste the line inside the list in `src/setup/release_keys.rs` (replace the
placeholder comment):

```rust
pub const RELEASE_PUBLIC_KEYS: &[[u8; 32]] = &[
    [0x00, /* ... 32 bytes in 0xNN form ... */ 0x00],
];
```

Keep the `0xNN` format: the publisher parses it to check that the signing key
is embedded. Ship a binary containing the key before (or in) the first release
you sign. Then delete the private key copies you do not need.

## 4. Publishing

Push a stable `vMAJOR.MINOR.PATCH` tag as before. The `plan` job runs without
secrets; the `publish` job then waits for approval of the `release`
environment. It requires, for the exact tag commit, a successful main CI run
and a successful `Installer e2e` run triggered by the push to `main` whose
`installer-e2e` job passed; it waits up to 30 minutes for each. A run started
by hand (`workflow_dispatch`, for example with `sabotage=wrong-hash`) or by a
pull request never counts, and the newest push run decides. It then installs
the pinned zipsign, signs every archive, writes `SHA256SUMS`, signs it as
`SHA256SUMS.sig`, uploads and re-downloads every asset (`SHA256SUMS.sig` last)
into a draft release, and attests build provenance for every archive. Only
then does the last step re-check the tag, both runs and the exact draft asset
list, and publish the draft. Verify an archive's provenance with
`gh attestation verify <archive> --repo norandom/mmhmem`.

If the installer e2e run for the tag commit failed, fix the cause on `main`
and tag the fixed commit; re-running the failed jobs until they pass and then
starting the release with `workflow_dispatch` (input `tag`) also works,
because the newest attempt of the push run is what counts.

## 5. Rotate keys

1. Generate a new pair as in step 1 (a new directory).
2. Add the NEW public key to `RELEASE_PUBLIC_KEYS` next to the old one and
   release a version signed with the OLD key, so existing installs learn both.
3. Replace the secret (`gh secret set MMHMEM_RELEASE_SIGNING_KEY --env release`
   with the new base64 value). Releases are now signed with the new key.
4. After clients have moved on, remove the old public key in a later release.
5. If the old key may be compromised, skip step 2's old-key release: ship a
   new binary through a channel you trust (a fresh install from the release
   page) and remove the old key at once.
