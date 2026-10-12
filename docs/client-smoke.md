# Client smoke test

Run this against an empty instance before calling a release done. It uses
the real npm, Cargo, Docker, Maven, NuGet, and Go clients. Each one
publishes into a new Forge and then reads that package back. It does not
add a format. PyPI, Helm, Conan, and generic files stay as they are.

The instance in this walkthrough is `http://127.0.0.1:3000`, signed in as
`admin`. **Settings** shows **Server version** `0.2.6` and the sidebar
says **FerroBox v0.2.6**. **0 B used** means the catalog is empty. A
directory install shows **Directory /var/lib/ferrobox/blobs**. A Garage
install shows **S3 bucket ferrobox**.

One token is enough. On **Security**, **Token name** `smoke`, leave
**Read** and **Write** unchecked, leave every repository unchecked, and
press **Create token**. The dialog title is **Copy the token now**. Copy
`fb_…` before **I have saved the token**. That secret is shown once. If
it leaks, **Revoke** it on the same page and create another.

**Package ecosystem** cannot be changed after create. On each Forge below,
set the ecosystem in the dialog before **Create repository**. The badge
on the repository has to name that ecosystem.

## npm

**Repositories**, **New repository**. **Name** `npm-smoke`. **Type**
`Forge`. **Package ecosystem** `npm`. **Create repository**. The toast
says `Repository "npm-smoke" created.` **Packages** says `Publish a
package with npm publish pointed at this registry.` The badge says
`npm`. Open **Set me up** and copy the registry URL. It ends with
`/npm/<UUID>/`, including the trailing slash.

In an empty directory:

```bash
mkdir -p "$HOME/smoke/npm" && cd "$HOME/smoke/npm"
cat > package.json <<'EOF'
{"name":"ferrobox-smoke","version":"0.1.0"}
EOF
echo "module.exports = {}" > index.js
cat > .npmrc <<EOF
registry=http://127.0.0.1:3000/npm/UUID/
//127.0.0.1:3000/npm/UUID/:_authToken=fb_…
EOF
npm publish
mkdir -p "$HOME/smoke/npm-install" && cd "$HOME/smoke/npm-install"
cat > .npmrc <<EOF
registry=http://127.0.0.1:3000/npm/UUID/
//127.0.0.1:3000/npm/UUID/:_authToken=fb_…
EOF
npm install ferrobox-smoke@0.1.0
```

`npm publish` prints the package name and version. `npm install` adds
`ferrobox-smoke` under `node_modules`. **Packages** lists
`ferrobox-smoke` `0.1.0`.

## Cargo

**New repository**. **Name** `cargo-smoke`. **Type** `Forge`. **Package
ecosystem** `cargo`. The toast says `Repository "cargo-smoke" created.`
**Packages** says `Publish a crate with cargo publish --registry
ferrobox, or upload a generic binary.` The badge says `cargo`.

```bash
mkdir -p "$HOME/.cargo"
cat >> "$HOME/.cargo/config.toml" <<'EOF'
[registries.ferrobox]
index = "sparse+http://127.0.0.1:3000/cargo/UUID/"
EOF
cat >> "$HOME/.cargo/credentials.toml" <<'EOF'
[registries.ferrobox]
token = "fb_…"
EOF
mkdir -p "$HOME/smoke/cargo/src" && cd "$HOME/smoke/cargo"
cat > Cargo.toml <<'EOF'
[package]
name = "ferrobox-smoke"
version = "0.1.0"
edition = "2021"
description = "FerroBox release check"
license = "MIT"
EOF
echo "pub fn smoke() {}" > src/lib.rs
cargo publish --registry ferrobox
mkdir -p "$HOME/smoke/cargo-install/src" && cd "$HOME/smoke/cargo-install"
cat > Cargo.toml <<'EOF'
[package]
name = "smoke-reader"
version = "0.1.0"
edition = "2021"
[dependencies]
ferrobox-smoke = { version = "0.1.0", registry = "ferrobox" }
EOF
echo "fn main() {}" > src/main.rs
cargo fetch
```

`cargo publish` ends with the crate name. `cargo fetch` downloads
`ferrobox-smoke`. **Packages** lists `ferrobox-smoke` `0.1.0`.

## Docker

**New repository**. **Name** `oci-smoke`. **Type** `Forge`. **Package
ecosystem** `oci`. The toast says `Repository "oci-smoke" created.`
**Packages** says `Publish an image with docker push pointed at this
registry.` The badge says `oci`.

Docker refuses plain HTTP until the daemon allows this host. Add
`127.0.0.1:3000` to `insecure-registries` in `/etc/docker/daemon.json`
and restart Docker. Restarting Docker stops the FerroBox container.
From `infra/`, `docker compose up -d` brings the same stack back. The
catalog is still there.

```bash
echo 'fb_…' | docker login 127.0.0.1:3000 -u __token__ --password-stdin
docker pull alpine:3.20
docker tag alpine:3.20 127.0.0.1:3000/UUID/demo:latest
docker push 127.0.0.1:3000/UUID/demo:latest
docker rmi 127.0.0.1:3000/UUID/demo:latest
docker pull 127.0.0.1:3000/UUID/demo:latest
```

`docker login` prints `Login Succeeded`. `docker push` ends with the
tag `latest`. **Packages** lists `demo` `latest`.

## Maven

**New repository**. **Name** `maven-smoke`. **Type** `Forge`. **Package
ecosystem** `maven`. The toast says `Repository "maven-smoke" created.`
**Packages** says `Publish a package with mvn deploy pointed at this
registry.` The badge says `maven`.

```bash
mkdir -p "$HOME/.m2" "$HOME/smoke/maven/src/main/java/org/example"
cat > "$HOME/.m2/settings.xml" <<'EOF'
<settings>
  <servers>
    <server>
      <id>ferrobox</id>
      <username>__token__</username>
      <password>fb_…</password>
    </server>
  </servers>
</settings>
EOF
cd "$HOME/smoke/maven"
cat > pom.xml <<'EOF'
<project>
  <modelVersion>4.0.0</modelVersion>
  <groupId>org.example</groupId>
  <artifactId>hello</artifactId>
  <version>1.0.0</version>
  <distributionManagement>
    <repository>
      <id>ferrobox</id>
      <url>http://127.0.0.1:3000/maven/UUID</url>
    </repository>
  </distributionManagement>
</project>
EOF
echo "package org.example; public class Hello {}" > src/main/java/org/example/Hello.java
mvn deploy
```

`mvn deploy` ends with `BUILD SUCCESS`. **Packages** lists `hello`
`1.0.0`.

## NuGet

**New repository**. **Name** `nuget-smoke`. **Type** `Forge`. **Package
ecosystem** `nuget`. The toast says `Repository "nuget-smoke" created.`
**Packages** says `Publish a package with dotnet nuget push pointed at
this registry.` The badge says `nuget`.

```bash
dotnet nuget add source http://127.0.0.1:3000/nuget/UUID/v3/index.json \
  -n ferrobox -u __token__ -p fb_… --store-password-in-clear-text
mkdir -p "$HOME/smoke/nuget" && cd "$HOME/smoke/nuget"
dotnet new classlib -n Hello.World --framework net8.0
dotnet pack Hello.World -o out
dotnet nuget push ./out/*.nupkg --source ferrobox --api-key fb_…
mkdir -p "$HOME/smoke/nuget-install" && cd "$HOME/smoke/nuget-install"
dotnet new console -n Reader --framework net8.0
dotnet add Reader package Hello.World --source ferrobox
```

`dotnet nuget push` prints `Your package was pushed.` **Packages**
lists `Hello.World` `1.0.0`.

## Go

**New repository**. **Name** `go-smoke`. **Type** `Forge`. **Package
ecosystem** `go`. The toast says `Repository "go-smoke" created.`
**Packages** says `Publish a module zip with curl pointed at this
GOPROXY.` The badge says `go`.

The Go tool reads modules. The write into a Forge is the zip the GOPROXY
protocol stores. `go get` is the client that has to succeed afterwards.

```bash
mkdir -p "$HOME/smoke/go" && cd "$HOME/smoke/go"
python3 - <<'PY'
import zipfile
prefix = "github.com/example/hello@v1.0.0"
with zipfile.ZipFile("hello.zip", "w") as z:
    z.writestr(prefix + "/go.mod", "module github.com/example/hello\n\ngo 1.22\n")
    z.writestr(prefix + "/hello.go", "package hello\n\nfunc Version() string { return \"v1.0.0\" }\n")
PY
curl -f -T hello.zip -H "Authorization: Bearer fb_…" \
  http://127.0.0.1:3000/go/UUID/github.com/example/hello/@v/v1.0.0.zip
mkdir -p "$HOME/smoke/go-install" && cd "$HOME/smoke/go-install"
go mod init smoke
GOPROXY=http://127.0.0.1:3000/go/UUID GONOSUMDB=* go get github.com/example/hello@v1.0.0
```

`curl` prints nothing and exits 0. `go get` adds the module to
`go.mod`. **Packages** lists `github.com/example/hello` `v1.0.0`.

## Done

**Settings** still shows the same server version and the same storage
line. **0 B used** has moved. Each of the six repositories has one
package on **Packages**. Deleting one repository does not delete the
others, and it does not delete the vulnerability feed.
