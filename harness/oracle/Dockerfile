# Rails oracle for the differential harness. Built from the vendored app so the
# comparison is against the exact pinned upstream commit.
FROM ruby:3.2.6-bookworm

ENV LANG=C.UTF-8 \
    BUNDLE_JOBS=4 \
    BUNDLE_PATH=/app/vendor/bundle \
    RAILS_ENV=production \
    RAILS_LOG_TO_STDOUT=1 \
    SECRET_KEY_BASE=oracle-secret-key-base-oracle-secret-key-base-0000 \
    DB_USERNAME=supermarket \
    DB_PASSWORD=supermarket \
    POSTGRES_IP=postgres \
    REDIS_URL=redis://redis:6379/0

RUN apt-get update && apt-get install -y --no-install-recommends \
      build-essential libpq-dev libarchive-dev libyaml-dev git nodejs \
      imagemagick shared-mime-info tzdata \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

COPY Gemfile Gemfile.lock ./
COPY engines/fieri engines/fieri
# The lockfile pins Bundler 2.3.7; the image ships a newer one.
RUN gem install bundler -v 2.3.7 \
    && bundle config set --local path vendor/bundle \
    && bundle _2.3.7_ install

COPY . .

# Asset precompilation is best effort: uglifier rejects the ES6 in the bundles,
# so it normally fails. `public/assets` is therefore seeded from the repo's
# prebuilt digest tree by a compose mount (`static/assets`); only the empty mount
# target is needed here.
RUN mkdir -p public/assets \
    && (bundle _2.3.7_ exec rails assets:precompile || echo "asset precompile failed")

EXPOSE 3001
CMD ["bundle", "_2.3.7_", "exec", "unicorn", "-l", "0.0.0.0:3001", "-p", "3001"]
