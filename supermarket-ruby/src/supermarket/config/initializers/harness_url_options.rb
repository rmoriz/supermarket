# Harness-only (not upstream): generated URLs use the request host by default,
# which would bake the oracle's listening port into every body. Pin them to
# FQDN/PORT/PROTOCOL, i.e. exactly what Supermarket::Host.full_url and the Rust
# port's `Config::base_url` derive, so both sides are comparable.
Rails.application.routes.default_url_options = {
  host: ENV["FQDN"],
  port: ENV["PORT"],
  protocol: ENV["PROTOCOL"] || "http",
}
