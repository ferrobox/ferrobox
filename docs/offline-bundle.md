# Offline bundle

One directory carries a FerroBox install onto a machine that cannot reach
a registry. You build it while you still have the network. You check the
signatures there. You carry the directory. The offline machine checks the
checksums and loads what is already in the files.

The directory holds:

- the FerroBox image for one version, as `docker save` wrote it
- `postgres:18-alpine`, the same way
- the Helm chart for that version
- one `ferrobox-osv-index` file, its `sha256sum` file, and `osv-sync.pub.pem`
- `SHA256SUMS` for every file in the directory

Garage is not in the bundle. The compose file starts PostgreSQL and
FerroBox with `STORAGE_BACKEND=filesystem`. A container scanner is not in
the bundle. The index is still npm, PyPI, crates.io, Maven, NuGet, and Go.

## Two signatures

The FerroBox image is signed with keyless Cosign. Checking that signature
asks Sigstore (Fulcio and Rekor) over the network. The bundle script does
that check before it saves the image, for the GitHub Actions identity of
`.github/workflows/release.yml` at the tag you named. An offline machine
cannot repeat that check. What it can do is `sha256sum -c SHA256SUMS`.

The vulnerability index is a Cosign **key** signature. The workflow signs
it with `--key` and `--tlog-upload=false`. The public key is
`osv-sync.pub.pem`. The bundle script checks that key, reads the manifest
digest from the signature, and pulls that digest. A tag that moves after
the check cannot swap the file. The offline import does not call the
registry. It sends the file to
`POST /api/security/osv-feed` with the hex digest in `X-FerroBox-Sha256`.
The **Sync now** button on the Vulnerability feed page pulls from the
registry, so it stays unused on the offline machine.

`postgres:18-alpine` is the upstream image. The script records the digest
it pulled. FerroBox does not sign that image.

Write down the fingerprint the script prints (`sha256sum SHA256SUMS`)
before you disconnect. On the other machine, the same command must print
the same fingerprint, and `sha256sum -c SHA256SUMS` must succeed.

## Build

From `infra/`, on a machine with Docker. Helm, oras, and cosign are used
when they are on `PATH`. Otherwise the script runs pinned images:
`ghcr.io/sigstore/cosign/cosign:v2.6.5`, `ghcr.io/oras-project/oras:v1.2.2`,
and `alpine/helm:3.16.4`. Docker pulls each one once.

```bash
./offline-bundle.sh --version 0.2.4 \
  --osv-ref ghcr.io/ferrobox/osv-db:2026-10-12 \
  --expect-ferrobox-digest sha256:cec6f297ba140724b6208d5118a893226bb1fb66d57616ad1920448684bb6591 \
  --out "$HOME/ferrobox-offline"
```

`--version` is the image tag without a leading `v`. `--osv-ref` is a date
tag, not a digest. `--expect-ferrobox-digest` is the digest on the GitHub
Release page for that version. The script stops if the pulled image is a
different digest.

The compose file in the bundle is the install that stays inside these
files. It starts PostgreSQL and FerroBox only. Docker copies the image
directory `/var/lib/ferrobox/blobs`, already owned by `nobody`, into a
fresh named volume, so the bundle does not add an image to change
ownership.

The chart is in the bundle for a cluster that already has the FerroBox
image, `postgres:18-alpine`, and `busybox:1.37`. The chart's init
containers use BusyBox to wait for PostgreSQL and to `chown` the volume.
A Kubernetes volume does not copy the image directory the way Docker
does. `garage.enabled=false` leaves out Garage and its bootstrap image.

```bash
helm install ferrobox chart/ferrobox-0.2.4.tgz \
  --namespace ferrobox --create-namespace \
  --set storage.backend=filesystem \
  --set garage.enabled=false \
  --set image.tag=0.2.4
```

The charts published for `0.2.4` and `0.2.5` still have `image.tag` set
to `0.2.2` in their values. Those tags were cut before the pin, and
`helm package` does not rewrite that field. `--set image.tag` selects
the image you loaded. `replicaCount` stays 1. Image tag `0.2.5` is the
release that contains this script. Its digest is
`sha256:609afb8c1ec94acc72a2c6142c9f5e983d88fa5c957d3185d37da2047eb2524a`.
**Settings** in that image still shows server version `0.2.2`.

## Install

Stop any FerroBox stack that already uses the container name `ferrobox`.

```bash
cd "$HOME/ferrobox-offline"
sha256sum SHA256SUMS
sha256sum -c SHA256SUMS
./install.sh
```

`install.sh` loads the two tar files and starts compose with
`pull_policy: never`. It writes `compose/.env` with `admin` / `admin` the
first time. Open http://127.0.0.1:3000.

The image tagged `0.2.4` was built from the commit that still prints
server version `0.2.2` on Settings. The tag and `bundle.json` are the
release identity. The line on Settings is the version compiled into that
commit.

Import the index from the bundle directory. Replace nothing in the digest:
it is the first field of `osv/ferrobox-osv-index.json.gz.sha256`.

```bash
SHA=$(awk 'NR==1 { print $1 }' osv/ferrobox-osv-index.json.gz.sha256)
TOKEN=$(curl -fsS http://127.0.0.1:3000/api/auth/login \
  -H 'content-type: application/json' \
  -d '{"username":"admin","password":"admin"}' \
  | python3 -c 'import json,sys; print(json.load(sys.stdin)["token"])')
curl -fsS http://127.0.0.1:3000/api/security/osv-feed \
  -H "Authorization: Bearer $TOKEN" \
  -H "X-FerroBox-Sha256: $SHA" \
  --data-binary @osv/ferrobox-osv-index.json.gz
```

The Vulnerability feed page shows that dataset and mode `file`. A
repository's Packages tab shows the same dataset. Deleting a repository
does not delete the feed.

## Empty instance

Stop any stack that already uses the container name `ferrobox`. The
commands below use image tag `0.2.4` and index `ghcr.io/ferrobox/osv-db:2026-10-12`.
The digest is the one printed on the GitHub Release for `v0.2.4`.

On the connected machine, from `infra/`:

```bash
./offline-bundle.sh --version 0.2.4 \
  --osv-ref ghcr.io/ferrobox/osv-db:2026-10-12 \
  --expect-ferrobox-digest sha256:cec6f297ba140724b6208d5118a893226bb1fb66d57616ad1920448684bb6591 \
  --out "$HOME/ferrobox-offline"
```

The script prints `sha256sum` of `SHA256SUMS`. Write that line down before
you disconnect. Copy the directory to the other machine.

On the offline machine:

```bash
cd "$HOME/ferrobox-offline"
sha256sum SHA256SUMS
sha256sum -c SHA256SUMS
./install.sh
```

The first command must print the line you wrote down. `sha256sum -c` must
end with no `FAILED` lines. `install.sh` loads the two archives and waits
until the container is healthy.

Open http://127.0.0.1:3000.

1. The sign-in page says **Sign in**. Username `admin`, password `admin`,
   then **Sign in**.
2. The sidebar says **FerroBox v0.2.2**. That string is compiled into the
   `0.2.4` image. The release identity is the image tag and the digest in
   `bundle.json`.
3. Open **Settings**. **Server version** is `0.2.2`. **Artifact storage**
   is `Directory /var/lib/ferrobox/blobs`.
4. Open **Vulnerability feed**. The page says
   `No vulnerability feed has been imported.` Do not press **Sync now**.
   That button pulls from the registry.
5. Import from the bundle directory. The login response contains a token.
   Keep it in the shell variable. If it appears in a screenshot or a log,
   revoke it from **Security**.

```bash
SHA=$(awk 'NR==1 { print $1 }' osv/ferrobox-osv-index.json.gz.sha256)
TOKEN=$(curl -fsS http://127.0.0.1:3000/api/auth/login \
  -H 'content-type: application/json' \
  -d '{"username":"admin","password":"admin"}' \
  | python3 -c 'import json,sys; print(json.load(sys.stdin)["token"])')
curl -fsS http://127.0.0.1:3000/api/security/osv-feed \
  -H "Authorization: Bearer $TOKEN" \
  -H "X-FerroBox-Sha256: $SHA" \
  --data-binary @osv/ferrobox-osv-index.json.gz
```

6. Reload **Vulnerability feed**. **Dataset** is the date inside the
   index. **Mode** is `file`. The empty sentence is gone.
7. Open **Repositories**, create any repository, and open **Packages**.
   The same dataset is listed. Deleting that repository leaves the feed
   in place: **Vulnerability feed** still shows **Mode** `file`.
