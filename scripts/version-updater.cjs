/**
 * Teaches commit-and-tag-version how to bump Cargo.toml and Cargo.lock.
 *
 * Cargo.toml: the `version = "x.y.z"` line of the `[package]` table only, so
 * dependency versions are never touched.
 * Cargo.lock: the `version` line of the `name = "apm"` package entry. Bumping it
 * here, in the same release commit, is what keeps `cargo build --locked` (CI and
 * release.yml) passing; a separate `cargo check` step could be forgotten.
 *
 * Both throw when their line is missing, so a reformat fails the release loudly
 * instead of tagging a version the binary does not report.
 * tests/release_assets.rs asserts the two files and `apm --version` agree.
 */
const LOCK = /(\[\[package\]\]\r?\nname = "apm"\r?\nversion = ")([^"]+)(")/;
const TOML = /(^\[package\][^[]*?^version\s*=\s*")([^"]+)(")/m;

const pattern = (contents) => {
  if (contents.includes("[[package]]")) {
    if (!LOCK.test(contents)) throw new Error('no apm package entry found in Cargo.lock');
    return LOCK;
  }
  if (!TOML.test(contents)) throw new Error("no [package] version found in Cargo.toml");
  return TOML;
};

module.exports.readVersion = (contents) => contents.match(pattern(contents))[2];

module.exports.writeVersion = (contents, version) =>
  contents.replace(pattern(contents), `$1${version}$3`);
