# Namespace placeholders

Defensive name reservations for the **`cairn-ehr`** identifier on the public package
registries, mirroring the domain/GitHub reservations. The bare word `cairn` was already
taken on PyPI, crates.io, and npm (unrelated projects), so `cairn-ehr` is the canonical
package name across all three — consistent with `cairn-ehr.org` and `github.com/hherb/cairn-ehr`.

These are **genuine placeholders for the real future home of the project**, not squats: each
points at the canonical site and repo and declares the project's actual (pre-release) status.
That is the accepted, low-risk way to hold a name — every registry forbids *squatting* names
you have no intent to use, but the legitimate owner publishing a stub is exactly what these
policies expect.

> **Status of these stubs:** version `0.0.0`, license `AGPL-3.0-only`. Replace with real
> releases when implementation begins. **crates.io publishes are permanent** (a crate can be
> *yanked* but never deleted), so get `crates/Cargo.toml` right before the first `cargo publish`.

## Publish (run yourself — needs your registry credentials)

### PyPI — `cairn-ehr`
```sh
cd packaging/pypi
uv build          # builds sdist + wheel into dist/
uv publish        # needs a PyPI API token (UV_PUBLISH_TOKEN or interactive)
```

### crates.io — `cairn-ehr`  (permanent once published)
```sh
cd packaging/crates
cargo publish --dry-run   # verify metadata first — this publish cannot be undone
cargo publish             # needs `cargo login <token>` (GitHub-linked, verified email)
```

### npm — `@cairn-ehr` scope
First create the free org `cairn-ehr` on npmjs.com (this is what actually reserves the scope),
then publish the stub so the scope is occupied:
```sh
cd packaging/npm
npm publish --access public   # scoped packages default to private; --access public is required
```
