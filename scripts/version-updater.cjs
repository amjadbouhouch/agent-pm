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
const VERSION = /^(version\s*=\s*")([^"]+)(")/;

/** Index of the version line in the table that starts at line `header`. */
const versionLine = (lines, header) => {
  for (let i = header + 1; i < lines.length && !lines[i].startsWith("["); i++) {
    if (VERSION.test(lines[i])) return i;
  }
  return -1;
};

const find = (contents) => {
  const lines = contents.split("\n");
  const lock = contents.includes("[[package]]");
  const header = lines.findIndex((l, i) =>
    lock
      ? l.trim() === "[[package]]" && (lines[i + 1] || "").trim() === 'name = "apm"'
      : l.trim() === "[package]",
  );
  const at = header < 0 ? -1 : versionLine(lines, header);
  if (at < 0) {
    throw new Error(lock ? "no apm package entry found in Cargo.lock" : "no [package] version found in Cargo.toml");
  }
  return { lines, at };
};

module.exports.readVersion = (contents) => {
  const { lines, at } = find(contents);
  return lines[at].match(VERSION)[2];
};

module.exports.writeVersion = (contents, version) => {
  const { lines, at } = find(contents);
  lines[at] = lines[at].replace(VERSION, `$1${version}$3`);
  return lines.join("\n");
};
