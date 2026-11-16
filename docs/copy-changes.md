# Public copy changes

The port drops brand words, logos and trademark lines from everything a visitor
sees, per the spec's *Copy and naming* rule. Wire contracts keep their wording:
API error codes, `Chef::Exceptions` messages, `provider=chef_oauth2`, JSON keys
and URLs are untouched.

`harness/normalize.rb` applies exactly these swaps to the Rails side before
diffing, so a swap listed here is the parity contract, not just a doc note.

## Text

| Upstream | Port |
|---|---|
| `Chef Supermarket` | `Supermarket` |
| `Chef's Supermarket`, `the Chef Supermarket`, `Supermarket by Chef` | `Supermarket` / `the Supermarket` |
| `Chef Infra Tool` / `Chef Infra Handler` (display label; the slug keeps `chef_infra_*`) | `Infra Tool` / `Infra Handler` |
| `Chef Infra Cookbooks` | `cookbooks` |
| `Chef Infra cookbook` | `cookbook` |
| `Chef Infra Client` | `client` |
| `Chef Infra clients` | `clients` |
| `Chef Infra Server` | `server` |
| `distribution in Chef Infra.` | `distribution.` |
| `Chef Server` | `server` |
| `Chef cookbook(s)` | `cookbook(s)` |
| `Chef tools and plugins` | `tools and plugins` |
| `Chef account` / `Chef Account` | `account` / `Account` |
| `Chef Community Slack` | `Community Slack` |
| `Chef Community Chat` | `Community Chat` |
| `Chef Mailing List` | `Mailing List` |
| `Read the Chef Blog` / `Read the Chef Docs` | `Read the Blog` / `Read the Docs` |
| `Learn Chef` | `Learn` |
| `Get Chef` | `Download` |
| `Chef Workstation` | `Workstation` |
| `Chef repo` | `repository` |
| `Chef Software, Inc.` | `the Supermarket maintainers` |
| `Chef's ecosystem` | `the ecosystem` |
| `Chef's open-source community platform` | `an open-source community platform` |
| `While Chef has` | `While the Supermarket maintainers have` |
| `contact Chef` | `contact the Supermarket maintainers` |
| `Chef Property Menu` (HTML comment) | `Property Menu` |
| `from the Chef community` | `from the community` |
| `Share Your Ideas for Improving Chef` | `Share Your Ideas for Improving Supermarket` |
| Copyright line naming Progress Software Corporation | `Copyright © {year} The Supermarket contributors.` |
| Trademark sentence naming Progress Software Corporation | generic "respective owners" sentence |
| `… reserved by their respective owners and their inclusion does not imply an endorsement, affiliation, or sponsorship as between Progress and the respective owners.` | `… reserved by their respective owners.` |

Kept deliberately:

- `Chef` as a person ("the awesome Chef who…", "reach this Chef"), and the
  counts label "Chefs".
- API messages: `is not a valid Chef version…`, `Chef::Exceptions::…`.
- Anything in a URL: `docs.chef.io`, `discourse.chef.io`, `community.chef.io`,
  `status.chef.io`, `chef/supermarket`, `progress.com`.

## Logos and icons

The branded image files are replaced with neutral `Supermarket` artwork, keeping
the upstream filenames and digests so the CSS, HTML and mail markup stay
byte-identical:

| Asset | Now shows |
|---|---|
| `supermarket-chef-logo-*.svg` / `*.png` | `Supermarket` wordmark |
| `chef-icon-*.png` | neutral `S` tile |
| `favicon-*.ico`, `apple-touch-icon-*.png` | same `S` tile |
| `logo_email-*.jpg` | `Supermarket` wordmark for mail |
