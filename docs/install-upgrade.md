# Install, upgrade, and rollback

FerroBox is one process. PostgreSQL holds the catalog. Artifact bytes live
in one place for the whole instance: an S3-compatible bucket, or a directory
on disk. Forge, Mirror, and Alloy are the repository kinds. The directory
is not a fourth kind.

Client protocols stay the same across 0.2.x releases.

## Install

### Docker Compose, with Garage (default)

```bash
cd infra
./dev-up.sh
```

The first run writes `infra/.env` from `infra/.env.example`. Open
http://localhost:3000 and sign in with the `ADMIN_USERNAME` and
`ADMIN_PASSWORD` in that file (`admin` / `admin` when the file was just
generated and not edited).

### Docker Compose, directory

Stop the Garage stack first. The container names are shared.

```bash
cd infra
docker compose down
./dev-up.sh --filesystem
```

PostgreSQL and FerroBox start. Garage does not. Bytes go to the
`ferrobox_blobs` volume, mounted at `/var/lib/ferrobox/blobs`.

### Helm

The published chart still defaults to PostgreSQL plus Garage:

```bash
helm install ferrobox oci://ghcr.io/ferrobox/charts/ferrobox --version 0.2.2 \
  --namespace ferrobox --create-namespace
```

That `--version` is the last published chart. This directory option ships
in the next chart release. Until that tag exists, render the chart from
this repository:

```bash
helm install ferrobox infra/chart --namespace ferrobox --create-namespace \
  --set storage.backend=filesystem \
  --set garage.enabled=false
```

`replicaCount` must stay `1` with a directory: two pods cannot share that
volume. Leave `storage.backend` unset (or `s3`) to keep the previous
install.

### What you should see

On **Settings**, under **Server version**:

- Garage install: `S3 bucket ferrobox` (the bucket name from the
  environment; the default is `ferrobox`).
- Directory install: `Directory /var/lib/ferrobox/blobs`.

The version string stays the published crate version until a new tag is
cut. A directory install is not a new repository type. Creating a Forge
still offers the same ecosystems.

## Check the image

Settings shows the version string compiled into the binary. The digest is
what you pin.

```bash
docker inspect --format '{{index .RepoDigests 0}}' ghcr.io/ferrobox/ferrobox:0.2.2
```

For the local quickstart image there is no registry digest. Record the
image id instead:

```bash
docker inspect --format '{{.Id}}' ferrobox:local
```

A byte you published is on the backend you just read in Settings, not in
the other one:

```bash
docker exec ferrobox find /var/lib/ferrobox/blobs -type f
```

That command is for a directory install. A Garage install stores the same
logical object in the bucket; the directory is empty or absent.

## Upgrade

1. Read this file and the changelog entry for the version you are moving to.
2. Dump PostgreSQL and snapshot the storage volume (the Garage data volume,
   the S3 bucket, or `ferrobox_blobs`) before you change the image.
3. Pin the new image by digest, not only by tag.
4. Start that image against the same `DATABASE_URL` and the same storage
   backend. The process applies pending migrations and then serves traffic.
5. Sign in and confirm **Settings**: the version string, and the same
   storage line as before the upgrade.

Do not change `STORAGE_BACKEND` as part of an upgrade. The database keeps
the object keys; the new backend does not contain the bytes.

## Rollback

Migrations run forward only, at startup. Replacing the process with the
previous image does not run them backwards. `helm rollback` has the same
limit: it restores the previous pod spec, not the previous schema.

Inside 0.2.x the contract is:

- A migration adds tables or columns.
- The previous binary does not read objects it does not know, so it still
  starts on the already-migrated database.
- Bytes already written stay where they were written.

A release that removes or renames a column is a major version. The way
back is the dump and the storage snapshot from step 2, restored together.
Starting the old image on a schema it cannot read is not a rollback.

Restarting the container, or recreating the pod on the same volume, is a
process restart. The catalog and the bytes are still there. It is not a
version rollback.

An offline copy of one version (image, PostgreSQL, chart, vulnerability
index, and the index public key) is described in
[offline-bundle.md](offline-bundle.md).

## ferrobox.io

The site is a separate repository. Do not edit it from this repository.

Image tag `0.2.4` contains directory storage. Inside that image, Settings
still shows server version `0.2.2` and the sidebar still says
`FerroBox v0.2.2`, because the tag was cut on the feature merge without
the version-pin commit. The site should name the image tag and the digest
on the GitHub Release. It should not say the Settings page prints `0.2.4`.

The offline bundle (`infra/offline-bundle.sh`) is not in a release tag
yet. Do not describe it on the site until the tag that contains it exists.

For the directory, which is already tagged:

- Bytes can live in a local directory or in S3. `./dev-up.sh` remains the
  Garage quickstart. `./dev-up.sh --filesystem` is PostgreSQL plus one
  directory.
- Keep Forge, Mirror, and Alloy as the only repository kinds.
- Name `0.2.4` as the image tag that contains the directory. Do not attach
  the feature to `0.2.2`.
- On the upgrade notes: pin the image by digest; 0.2.x migrations add
  tables and columns; rolling the process back does not rewind
  PostgreSQL; a breaking schema change is a major version and the way
  back is a dump plus the storage volume; switching storage backends
  does not copy bytes.
- Do not add distro packages, a container scanner, or a fourth repository
  kind.
