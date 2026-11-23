ENV["BUNDLE_GEMFILE"] ||= File.expand_path("../Gemfile", __dir__)

require "bundler/setup" # Set up gems listed in the Gemfile.

# compass 1.0.3 still calls File.exists?, which Ruby removed in 3.2
class << File
  alias_method :exists?, :exist? unless File.respond_to?(:exists?)
end

# rack 3 moved the handlers into the rackup gem, but unicorn-rails still
# registers itself on Rack::Handler
require "rack"
require "rackup/handler"
module Rackup
  module Handler
    Rack = ::Rack # so the "Rack::Handler::Unicorn" registrations resolve
  end
end
Rack::Handler = Rackup::Handler unless Rack.const_defined?(:Handler, false)
