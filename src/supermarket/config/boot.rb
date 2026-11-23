ENV["BUNDLE_GEMFILE"] ||= File.expand_path("../Gemfile", __dir__)

require "bundler/setup" # Set up gems listed in the Gemfile.

# compass 1.0.3 still calls File.exists?, which Ruby removed in 3.2
class << File
  alias_method :exists?, :exist? unless File.respond_to?(:exists?)
end
