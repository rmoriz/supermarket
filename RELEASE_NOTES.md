# Unreleased

Dependency and maintenance work for Supermarket.

## Security fixes

`bundle-audit` reports no vulnerabilities for the application and the
fieri engine after this branch. The npm audit is clean too.

- **Rails 8.1.4.1** (from 7.2.4): Active Storage arbitrary file write/read
  and RCE through video previews and image analysis, Strong Parameters
  bypass in `Parameters#each_key`, XSS in Action View text/tag/highlight
  helpers and `strip_tags`, Active Support `safe_constantize` DoS,
  session restoration and missing SameSite handling in the cache store,
  Session Fixation in the cookie jar, the 8.1.4.1 advisories and a batch
  of lower-severity Action Pack, Action Mailbox and Active Storage
  issues.
- **rack-protection 4.2.2** (from 3.0.5, via rack 3): a crafted request
  made CookieTossing reject or drop the session of every later request.
- **rubyzip 3.7.0** (from 2.4.1): path traversal when expanding archives.
- **css_parser 3.4.0** (from 1.11.0, with premailer 1.33.0): missing
  certificate validation, SSRF and local file disclosure through
  `read_remote_file` and `@import`.
- **excon 1.12.0**: additional sensitive headers are redacted on
  redirects.
- Transitive gems: addressable, net-imap, net-ssh, net-smtp, jwt,
  websocket-driver, yard, ruby-lsp, rubyntlm, rack-test, loofah,
  rails-html-sanitizer, Rack::Session, rackup, uri and others were moved
  to their current releases.

## Upgrades that carry the fixes

- Ruby 4.0.10 in CI, in the `.ruby-version` files and in the omnibus
  package (3.3.x before). `ddtrace` 1.x stops below 3.4 and is replaced
  by `datadog` 2.49, whose tracer supersedes it and follows the current
  configuration API.
- Standard library gems that Ruby 4.0 no longer bundles (`ostruct`,
  `logger`, `observer`, `rexml`, `csv` and friends) are declared
  explicitly.
- Rails 8.1.4.1 for the app and the fieri engine; the cache store moves
  from the retired `redis-rails` to the built-in `:redis_cache_store`
  with the `redis` gem. The feature flags move from rollout 2.5 to
  rollout 3.3 with `rollout-redis-adapter`.
- sidekiq 7.3.10, which rack 3 requires. The removed delayed extensions
  come back through `sidekiq-delay_extensions`. `REDIS_URL` is normalised
  to a database index, because redis-client no longer accepts the
  `/<db>/<namespace>` suffix. Sidekiq 7 needs Redis 6.2 or newer.
- rack 3.2.9 and rack-protection 4.2.2. `boot.rb` maps `Rack::Handler`
  onto the rackup handler registry for the unmaintained unicorn-rails.
- chef 19.4.46 (from 18.5.0) so inspec-core 7 and chef-winrm-fs 1.4.4 can
  drop their rubyzip pin.
- Build tooling: test-kitchen 4.1.5, kitchen-vagrant 2.4.1, excon 1.12.0
  and current omnibus/omnibus-software revisions.
- The rest of the bundles were refreshed as well: aws-sdk, dry-*, kaminari,
  redcarpet, puma, rspec, capybara, factory_bot, shoulda-matchers, vcr,
  webmock, octokit, dotenv, premailer and the linters.

## Other changes

- Quality metric feedback, cookbook documents that are not markdown and
  the announcement banner are handed to the view already escaped, and
  fieri evaluates job payloads through a plain hash. The layouts and the
  atom/mailer templates no longer need to mark output as safe by hand.
- fieri runs cookstyle against the configuration that ships with
  cookstyle instead of whatever `.rubocop.yml` the uploaded cookbook
  carries, which makes the metric independent of the artifact.
- `chefstyle` is replaced by `cookstyle` for linting, since chefstyle is
  frozen on an old RuboCop. CI, CONTRIBUTING.md and the local setup use
  cookstyle 9.0.9 with rubocop 1.91 and rubocop-rails 2.38.
- The unused IRC log redirects (`/chat`, `IrcLogsController`) are gone.
- `license_finder` left the development group because it still pins
  rubyzip below 3 and no task uses it. Install it ad hoc, as
  CONTRIBUTING.md describes.
- fieri rejects job payloads that are not plain JSON types.
- `rspec-rails` 8.1 in the app and the fieri engine (from 6.1 and 7.1).
- `uglifier` cannot parse the ES6 in the bundled editor JavaScript and
  broke `assets:precompile` in production. `terser` replaces it, so a
  package build now needs a JavaScript runtime.

## Package build

- The package ships PostgreSQL 17.12 instead of 13.22. The 13 binaries
  stay in the package so the existing `pg_upgrade` resource can migrate a
  cluster on the first `reconfigure` after the upgrade; `postgresql.conf`
  holds no settings that 17 dropped. The license notice and the version
  table in the upgrade documentation follow.
- Redis 8.12.0 (from 7.0.8) with its own project software definition,
  since Redis 8 is tri-licensed in `LICENSE.txt` and the omnibus-software
  definition still names BSD-3-Clause.
- The ctl command bundle moves from chef 16.18 to chef 18.11.11. Chef 16
  crashed on Ruby 4 with `File.exists?` while loading cookbooks. The
  cookbooks now declare `chef_version >= 18`.
- berkshelf 8.1.31, which requires `minitar` instead of the removed
  `archive/tar/minitar` path.
- openssl 3.2.6, libarchive 3.8.1, file 5.48 and the CA bundle from
  2026-11-05.
- The build lab installs nodejs for the asset step, the kitchen platforms
  move to Ubuntu 22.04/24.04 and AlmaLinux 9, and the expeditor release
  matrix drops Ubuntu 18.04/20.04 for 22.04/24.04.
- The packaging cookbooks pass cookstyle again.

## Breaking or noteworthy changes

- Ruby 4.0 needed a full bundle refresh for the standard library gems
  listed above.
- fieri no longer pins json below 3; Rails 8.1 stopped passing the
  `quirks_mode` keyword.
- The omnibus default `redis_url` drops the unused `/supermarket` suffix.
  Existing installations with a custom URL need nothing, sidekiq strips
  the suffix itself.
- The PostgreSQL 13 to 17 migration in the package copies the data
  directory and needs the downtime and free disk space of a dump and
  restore. Raise `pg_upgrade_timeout` beforehand for large databases.
- A custom software definition or override that relies on chef 16 in the
  ctl commands has to move to chef 18.

## Still open

- The omnibus package pins Ruby 4.0.10. omnibus-software has no entry
  for that release yet, so the project override carries the source
  checksum. A full package build has not run here.
- `aws-sdk-s3` 1.116.0 in the omnibus bundle: omnibus main pins it below
  1.117 (S3 encryption client key commitment).
- `addressable` 2.8.10 and `rubyzip` 2.4.1 in the omnibus bundle:
  omnibus pins ohai below 19 and therefore chef 18.5, and chef-zero below
  chef 19 requires rubyzip below 3. Only the build environment is
  affected, the packaged ctl commands run chef 18.11.
- The rubyzip 2.4.1 pin above also covers the path traversal fix; worth
  revisiting once chef-zero allows 3.
- PostgreSQL 13.22 stays in the package as the upgrade source and is
  past its end of life; it disappears once 13 as an upgrade path is
  dropped.
