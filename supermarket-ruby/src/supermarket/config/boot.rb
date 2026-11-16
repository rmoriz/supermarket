ENV["BUNDLE_GEMFILE"] ||= File.expand_path("../Gemfile", __dir__)

# Ruby 3.2 removed File.exists?; compass 1.0.3 still calls it during boot.
class << File
  alias_method :exists?, :exist? unless File.respond_to?(:exists?)
end

require "bundler/setup" # Set up gems listed in the Gemfile.
