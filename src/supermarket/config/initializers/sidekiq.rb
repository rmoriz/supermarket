redis_for_job_queue = ENV["REDIS_JOBQ_URL"].presence ||
                      ENV["REDIS_URL"].presence ||
                      "redis://localhost:6379/0/supermarket"

# Sidekiq 7 reads the URL with redis-client, which cannot parse the legacy
# "<db>/<namespace>" suffix and only understands the database index.
redis_for_job_queue = redis_for_job_queue.sub(%r{(:\d+/\d+)/.+\z}, '\1')

Sidekiq.configure_server do |config|
  config.redis = { url: redis_for_job_queue }

  # make Sidekiq load the schedule, because Redis
  # probably reachable at this point
  Sidekiq::Cron::Job.load_from_hash(
    "Daily refresh of the sitemap" => {
      class: "SitemapRefreshWorker",
      cron: "@daily",
    },
    "Schedule refresh of expiring tokens" => {
      class: "OauthTokenRefreshScheduleWorker",
      cron: "*/5 * * * *",
    }
  )
end

Sidekiq.configure_client do |config|
  config.redis = { url: redis_for_job_queue }
end

# Sidekiq removed the Delayed Extensions in 7.0. Keep .delay calls working
# through the extracted gem until they are migrated to ActiveJob classes.
Sidekiq::DelayExtensions.enable_delay!

# Jobs enqueued before the upgrade still carry the old class names.
module Sidekiq::Extensions
end
Sidekiq::Extensions::DelayedClass = Sidekiq::DelayExtensions::DelayedClass
Sidekiq::Extensions::DelayedModel = Sidekiq::DelayExtensions::DelayedModel
Sidekiq::Extensions::DelayedMailer = Sidekiq::DelayExtensions::DelayedMailer
