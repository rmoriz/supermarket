# frozen_string_literal: true

module Supermarket
  #
  # Helpers for the legacy <tt>/<db>/<namespace></tt> REDIS_URL form.
  #
  # Sidekiq 7 and the redis gem 5 resolve URLs with redis-client, which only
  # understands a database index and rejects the namespace suffix. Namespacing
  # is done by the callers where it is needed, so the URL is reduced to the
  # connection details.
  #
  module RedisUrl
    module_function

    #
    # Drops a <tt>/<namespace></tt> suffix from the given URL
    #
    # @param url [String] the Redis URL
    #
    # @return [String] the URL without the namespace
    #
    def normalize(url)
      url.to_s.sub(%r{(:\d+/\d+)/.+\z}, '\1')
    end
  end
end
