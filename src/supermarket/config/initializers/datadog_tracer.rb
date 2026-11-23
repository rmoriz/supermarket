# Disable/Enable and configure the datadog application tracer
# crf. https://docs.datadoghq.com/tracing/trace_collection/dd_libraries/ruby/

if ENV["DATADOG_TRACER_ENABLED"] && ENV["DATADOG_TRACER_ENABLED"] == "true"
  require "datadog/auto_instrument"

  Datadog.configure do |c|
    c.service = ENV["DATADOG_APP_NAME"] || "rails_app"
    c.env = ENV["DATADOG_ENVIRONMENT"] if ENV["DATADOG_ENVIRONMENT"]
  end
end
