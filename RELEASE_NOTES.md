# Unreleased

Dependency and security maintenance for Supermarket. No feature changes.

## Security fixes

`bundle-audit` reports no vulnerabilities for the application and the
fieri engine after this branch. The npm audit is clean too.

- **Rails 8.0.5.2** (from 7.2.4): Active Storage arbitrary file write/read
  and RCE through video previews and image analysis, Strong Parameters
  bypass in `Parameters#each_key`, XSS in Action View text/tag/highlight
  helpers and `strip_tags`, Active Support `safe_constantize` DoS,
  session restoration and missing SameSite handling in the cache store,
  Session Fixation in the cookie jar and a batch of lower-severity
  Action Pack, Action Mailbox and Active Storage issues.
- **rack-protection 4.2.2** (from 3.0.5, via rack 3): a crafted request
  made CookieTossing reject or drop the session of every later request.
- **rubyzip 3.7.0** (from 2.4.1): path traversal when expanding archives.
- **css_parser 3.4.0** (from 1.11.0, with premailer 1.27.0): missing
  certificate validation, SSRF and local file disclosure through
  `read_remote_file` and `@import`.
- **excon 1.12.0**: additional sensitive headers are redacted on
  redirects.
- Transitive gems: addressable, net-imap, net-ssh, net-smtp, jwt,
  websocket-driver, yard, ruby-lsp, rubyntlm, rack-test, loofah,
  rails-html-sanitizer, Rack::Session, rackup, uri, json and others were
  moved to their current releases.

## Upgrades that carry the fixes

- Ruby 3.3.9 in CI and in the omnibus package (3.1.5 before): nokogiri
  1.19 already needs 3.2+, while ddtrace 1.x stops below 3.4.
- Rails 8.0.5.2 for the app and the fieri engine; the cache store moves
  from the retired `redis-rails` to the built-in `:redis_cache_store`.
- sidekiq 7.3.10, which rack 3 requires. The removed delayed extensions
  come back through `sidekiq-delay_extensions`. `REDIS_URL` is normalised
  to a database index, because redis-client no longer accepts the
  `/<db>/<namespace>` suffix. Sidekiq 7 needs Redis 6.2 or newer.
- rack 3.2.9 and rack-protection 4.2.2. `boot.rb` maps `Rack::Handler`
  onto the rackup handler registry for the unmaintained unicorn-rails.
- chef 19.4.46 (from 18.5.0) so inspec-core 7 and chef-winrm-fs 1.4.4 can
  drop their rubyzip pin.
- Build tooling: test-kitchen 4.1.5, kitchen-vagrant 2.4.1, cookstyle
  9.0.9, excon 1.12.0 and current omnibus/omnibus-software revisions.

## Breaking or noteworthy changes

- `license_finder` left the development group because it still pins
  rubyzip below 3 and no task uses it. Install it ad hoc, as
  CONTRIBUTING.md now describes.
- fieri rejects job payloads that are not plain JSON types and pins json
  to 2.x, since json 3 removed the `quirks_mode` keyword that Active
  Support 8.0 still passes.
- The omnibus default `redis_url` drops the unused `/supermarket` suffix.
  Existing installations with a custom URL need nothing, sidekiq strips
  the suffix itself.

## Still open

Blocked upstream, no released fix path yet:

- `aws-sdk-s3` 1.116.0 in the omnibus bundle: omnibus main pins it below
  1.117 (S3 encryption client key commitment).
- `addressable` 2.8.10 and `rubyzip` 2.4.1 in the omnibus bundle:
  chef-zero requires rubyzip below 3, and chef 18.5 cannot be bumped to
  19 there because omnibus main pins ohai below 19 and chef 18.6 pins ffi
  below 1.17 while omnibus-software needs 1.17.
