#!/usr/bin/env ruby
# Response normalization for the differential harness.
#
# Only differences that cannot be contractually equal are erased: clock readings,
# generated ids, CSRF tokens, ETags, cookie ciphertext and known copy swaps. A
# normalizer that swallows real differences is a harness bug, so every rule is
# narrow and the tests in spec/normalize_test.rb pin the boundaries.

require "json"

# json >= 3 dropped the `quirks_mode:` generator option that ActiveSupport 7.2
# still passes when serializing cookies.
if JSON::VERSION.to_i >= 3
  module JSON
    class << self
      alias_method :generate_with_quirks, :generate
      def generate(obj, opts = nil, *rest)
        opts = opts.reject { |key, _| key == :quirks_mode } if opts.is_a?(Hash)
        generate_with_quirks(obj, opts, *rest)
      end

      alias_method :parse_with_quirks, :parse
      def parse(source, opts = nil, **kwargs)
        opts = (opts || {}).merge(kwargs)
        opts = opts.reject { |key, _| key == :quirks_mode }
        parse_with_quirks(source, **opts)
      end
    end
  end
end

module Normalize
  module_function

  COOKIE_NAME = "_supermarket_session"

  # Copy swaps from docs/copy-changes.md, applied to the Rails side only.
  COPY_SWAPS = [
    ["Chef Supermarket", "Supermarket"],
    ["Chef's Supermarket", "Supermarket"],
    ["the Chef Supermarket", "the Supermarket"],
    ["Supermarket by Chef", "Supermarket"],
  ].freeze

  TIMESTAMP = /\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:\.\d+)?(?:Z|[+-]\d{2}:\d{2})/
  RAILS_TIMESTAMP = /\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2}(?:\.\d+)? UTC/
  ETAG = /"?[0-9a-f]{16,}"?/
  CSRF = /name="csrf-token" content="[^"]*"/
  AUTH_TOKEN = /name="authenticity_token" value="[^"]*"/
  NONCE = /'nonce-[^']+'/
  OAUTH_STATE = /state=[0-9a-f]{20,}/

  # Fields whose value is a timestamp or generated id, matched by JSON key.
  VOLATILE_KEYS = %w[
    created_at updated_at published_at oauth_expires tarball_updated_at
    request_id expires_at
  ].freeze

  # Redis counters that move with every connection, even between two back-to-back
  # requests (and the Rust app and the oracle hold separate connections).
  LIVE_COUNTERS = %w[
    connected_clients used_memory used_memory_peak uptime_in_seconds
    waiting_on_lock connections
  ].freeze

  def body(text, content_type: "")
    return nil if text.nil?
    return text unless content_type.include?("json")

    document = JSON.parse(text)
    text = JSON.dump(scrub_json(document))
    text
  rescue JSON::ParserError
    text
  end

  def scrub_json(value)
    case value
    when Hash
      value.each_with_object({}) do |(key, inner), out|
        out[key] =
          if VOLATILE_KEYS.include?(key) && !inner.nil?
            "<timestamp>"
          elsif LIVE_COUNTERS.include?(key) && !inner.nil?
            "<live>"
          elsif key == "csrf" || key == "csrf-token"
            "<csrf>"
          else
            scrub_json(inner)
          end
      end
    when Array then value.map { |inner| scrub_json(inner) }
    else value
    end
  end

  def html(text)
    return nil if text.nil?

    text = text.gsub(CSRF, 'name="csrf-token" content="<csrf>"')
    text = text.gsub(AUTH_TOKEN, 'name="authenticity_token" value="<csrf>"')

    # Whitespace between tags is not contractual: templates may place a control
    # tag where ERB emitted a newline. Drop whitespace-only text nodes and
    # collapse runs of whitespace inside text, keeping the tag stream comparable.
    out = +""
    text.scan(/(<[^>]*>)|([^<]+)/) do |tag, content|
      if tag
        out << normalize_tag(tag)
      else
        collapsed = content.gsub(/\s+/, " ").strip
        out << collapsed unless collapsed.empty?
      end
    end
    out
  end

  def normalize_tag(tag)
    opening = tag.sub(/>\z/, "")
    name = opening[/\A<[\w-]+/]
    return tag unless name

    attrs = opening[name.length..].scan(/[\w:-]+(?:="[^"]*")?/).sort
    "#{name} #{attrs.join(' ')}>".squeeze(" ").sub(" >", ">")
  end

  # Applies the swap table to any textual body.
  def copy(text)
    return nil if text.nil?

    COPY_SWAPS.reduce(text) { |acc, (from, to)| acc.gsub(from, to) }
  end

  # Transport details that differ by server (unicorn closes, axum keeps alive and
  # may compress) are not part of the contract.
  DROPPED_HEADERS = %w[connection transfer-encoding vary content-length x-runtime].freeze

  def headers(headers)
    headers.each_with_object({}) do |(name, value), out|
      key = name.downcase
      next if DROPPED_HEADERS.include?(key)

      case key
      when "set-cookie"
        out[key] = Array(value).map { |cookie| normalize_cookie(cookie) }
      when "etag" then out[key] = "<etag>"
      when "x-request-id" then out[key] = "<request-id>"
      when "date", "expires" then out[key] = "<date>"
      when "content-security-policy" then out[key] = value.gsub(NONCE, "'nonce-<nonce>'")
      when "content-length" then next
      when "location" then out[key] = origin(value).gsub(/state=[0-9a-f]+/, "state=<state>")
      else out[key] = origin(value)
      end
    end
  end

  def normalize_cookie(cookie)
    cookie.sub(/#{Regexp.escape(COOKIE_NAME)}=([^;]*)/) { "#{COOKIE_NAME}=<ciphertext>" }
  end

  # The oracle listens on 3001 while both sides are configured for PORT 3005;
  # rewrite its origin so absolute URLs are comparable.
  ORACLE_ORIGIN = %r{https?://127\.0\.0\.1:3001}.freeze
  TARGET_ORIGIN = "http://localhost:3005".freeze

  def origin(text)
    return nil if text.nil?

    text.gsub(ORACLE_ORIGIN, TARGET_ORIGIN)
  end

  # Full response normal form.
  def response(status:, headers:, raw_body:)
    content_type = Array(headers).to_h { |k, v| [k.downcase, v] }["content-type"].to_s
    normalized = copy(body(origin(raw_body.to_s), content_type: content_type))
    normalized = html(normalized) if content_type.include?("html")
    normalized = normalized&.gsub(TIMESTAMP, "<timestamp>")&.gsub(RAILS_TIMESTAMP, "<timestamp>")
    normalized = normalized&.gsub(OAUTH_STATE, "state=<state>")
    {
      "status" => status,
      "headers" => headers(headers),
      "body" => normalized,
    }
  end
end
