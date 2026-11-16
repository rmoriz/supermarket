# Upstream source

This directory is https://github.com/chef/supermarket, imported verbatim apart
from this file into `supermarket-rs` as a read-only reference for the Rust port.

- Upstream: https://github.com/chef/supermarket
- Pinned commit: `80d517b228fac3290fc83b06d3a873117d13b090` (upstream `main`,
  2026-07-29)
- Imported: 2026-11-16, full history, via `git subtree`
- Licence: Apache License 2.0, see [`LICENSE`](LICENSE) in this directory

**Do not edit upstream files here.** The Rails application lives in
`src/supermarket/`. `UPSTREAM.md` is the one file in this directory that is ours.
Make changes upstream, then pull them in:

```bash
git remote add upstream-supermarket https://github.com/chef/supermarket  # once per clone
git fetch --no-tags upstream-supermarket
git subtree pull --prefix=supermarket-ruby upstream-supermarket main
```

After a pull, update the pinned commit above — `UPSTREAM.md` is the exception to
the rule above. Keeping the pin current is what makes the port's diffs against
upstream meaningful. If upstream ever adds its own `UPSTREAM.md`, resolve that
add/add conflict in favour of this file.
