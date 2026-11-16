# supermarket-rs

A Rust port workspace for [Chef Supermarket](https://github.com/chef/supermarket),
the community site for Chef cookbooks.

`supermarket-ruby/` holds the upstream Ruby on Rails application, imported
verbatim at a pinned commit. It is a **read-only reference**: browse it, grep
it, blame it, but never edit files inside it here. Changes belong upstream or in
the Rust port.

- Provenance, pinned commit, and licence: [`supermarket-ruby/UPSTREAM.md`](supermarket-ruby/UPSTREAM.md)
- Design: [`docs/superpowers/specs/2026-11-16-vendor-supermarket-ruby-design.md`](docs/superpowers/specs/2026-11-16-vendor-supermarket-ruby-design.md)
- Plan: [`docs/superpowers/plans/2026-11-16-vendor-supermarket-ruby.md`](docs/superpowers/plans/2026-11-16-vendor-supermarket-ruby.md)

## Updating the vendored source

The import is a `git subtree`, so it can be advanced without losing history:

```bash
git fetch upstream-supermarket
git subtree pull --prefix=supermarket-ruby upstream-supermarket main
```

Then update the pinned commit in `supermarket-ruby/UPSTREAM.md`.

## Licensing

`supermarket-ruby/` is upstream's code, unchanged, under the Apache License 2.0
in [`supermarket-ruby/LICENSE`](supermarket-ruby/LICENSE). Everything else in
this repository is the port's own work.
