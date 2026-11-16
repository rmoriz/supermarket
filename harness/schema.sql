CREATE EXTENSION IF NOT EXISTS "pg_trgm";
CREATE EXTENSION IF NOT EXISTS "plpgsql";
CREATE TABLE "accounts" (
  "id" serial PRIMARY KEY,
  "user_id" integer,
  "uid" varchar,
  "username" varchar,
  "oauth_token" varchar,
  "oauth_secret" varchar,
  "oauth_expires" timestamp,
  "provider" varchar,
  "oauth_refresh_token" varchar
);
CREATE INDEX "index_accounts_on_oauth_expires" ON "accounts" ("oauth_expires");
CREATE UNIQUE INDEX "index_accounts_on_uid_and_provider" ON "accounts" ("uid", "provider");
CREATE INDEX "index_accounts_on_uid" ON "accounts" ("uid");
CREATE INDEX "index_accounts_on_user_id" ON "accounts" ("user_id");
CREATE UNIQUE INDEX "index_accounts_on_username_and_provider" ON "accounts" ("username", "provider");
CREATE INDEX "index_accounts_on_username" ON "accounts" ("username");
CREATE TABLE "categories" (
  "id" serial PRIMARY KEY,
  "name" varchar,
  "created_at" timestamp,
  "updated_at" timestamp,
  "slug" varchar
);
CREATE INDEX "index_categories_on_slug" ON "categories" ("slug");
CREATE TABLE "collaborators" (
  "id" serial PRIMARY KEY,
  "resourceable_id" integer,
  "user_id" integer,
  "created_at" timestamp,
  "updated_at" timestamp,
  "resourceable_type" varchar,
  "group_id" integer
);
CREATE UNIQUE INDEX "index_collaborators_on_user_id_and_group_id_and_resourceable" ON "collaborators" ("user_id", "resourceable_type", "resourceable_id", "group_id");
CREATE TABLE "contributor_request_responses" (
  "id" serial PRIMARY KEY,
  "contributor_request_id" integer NOT NULL,
  "decision" varchar NOT NULL,
  "created_at" timestamp,
  "updated_at" timestamp
);
CREATE UNIQUE INDEX "index_contributor_request_responses_on_contributor_request_id" ON "contributor_request_responses" ("contributor_request_id");
CREATE TABLE "contributor_requests" (
  "id" serial PRIMARY KEY,
  "organization_id" integer NOT NULL,
  "user_id" integer NOT NULL,
  "created_at" timestamp,
  "updated_at" timestamp,
  "ccla_signature_id" integer NOT NULL
);
CREATE UNIQUE INDEX "index_contributor_requests_on_organization_id_and_user_id" ON "contributor_requests" ("organization_id", "user_id");
CREATE TABLE "contributors" (
  "id" serial PRIMARY KEY,
  "user_id" integer,
  "organization_id" integer,
  "admin" boolean,
  "created_at" timestamp,
  "updated_at" timestamp
);
CREATE INDEX "index_contributors_on_organization_id" ON "contributors" ("organization_id");
CREATE UNIQUE INDEX "index_contributors_on_user_id_and_organization_id" ON "contributors" ("user_id", "organization_id");
CREATE INDEX "index_contributors_on_user_id" ON "contributors" ("user_id");
CREATE TABLE "cookbook_dependencies" (
  "id" serial PRIMARY KEY,
  "name" varchar NOT NULL,
  "version_constraint" varchar DEFAULT '>= 0.0.0' NOT NULL,
  "cookbook_version_id" integer NOT NULL,
  "created_at" timestamp,
  "updated_at" timestamp,
  "cookbook_id" integer
);
CREATE INDEX "index_cookbook_dependencies_on_cookbook_id" ON "cookbook_dependencies" ("cookbook_id");
CREATE UNIQUE INDEX "cookbook_dependencies_unique_by_name_and_constraint" ON "cookbook_dependencies" ("cookbook_version_id", "name", "version_constraint");
CREATE INDEX "index_cookbook_dependencies_on_cookbook_version_id" ON "cookbook_dependencies" ("cookbook_version_id");
CREATE TABLE "cookbook_followers" (
  "id" serial PRIMARY KEY,
  "cookbook_id" integer NOT NULL,
  "user_id" integer NOT NULL,
  "created_at" timestamp,
  "updated_at" timestamp
);
CREATE UNIQUE INDEX "index_cookbook_followers_on_cookbook_id_and_user_id" ON "cookbook_followers" ("cookbook_id", "user_id");
CREATE TABLE "cookbook_version_platforms" (
  "id" serial PRIMARY KEY,
  "cookbook_version_id" integer,
  "supported_platform_id" integer,
  "created_at" timestamp,
  "updated_at" timestamp
);
CREATE UNIQUE INDEX "index_cvp_on_cvi_and_spi" ON "cookbook_version_platforms" ("cookbook_version_id", "supported_platform_id");
CREATE TABLE "cookbook_versions" (
  "id" serial PRIMARY KEY,
  "cookbook_id" integer,
  "license" varchar,
  "version" varchar,
  "created_at" timestamp,
  "updated_at" timestamp,
  "tarball_file_name" varchar,
  "tarball_content_type" varchar,
  "tarball_file_size" integer,
  "tarball_updated_at" timestamp,
  "readme" text DEFAULT '' NOT NULL,
  "readme_extension" varchar DEFAULT '' NOT NULL,
  "dependencies_imported" boolean DEFAULT FALSE,
  "description" text,
  "legacy_id" integer,
  "web_download_count" integer DEFAULT 0,
  "api_download_count" integer DEFAULT 0,
  "changelog" text,
  "changelog_extension" varchar DEFAULT '' NOT NULL,
  "chef_versions" json,
  "ohai_versions" json,
  "user_id" integer,
  "spdx_license_url" varchar
);
CREATE UNIQUE INDEX "index_cookbook_versions_on_legacy_id" ON "cookbook_versions" ("legacy_id");
CREATE UNIQUE INDEX "index_cookbook_versions_on_version_and_cookbook_id" ON "cookbook_versions" ("version", "cookbook_id");
CREATE INDEX "index_cookbook_versions_on_version" ON "cookbook_versions" ("version");
CREATE TABLE "cookbooks" (
  "id" serial PRIMARY KEY,
  "name" varchar NOT NULL,
  "created_at" timestamp,
  "updated_at" timestamp,
  "source_url" varchar,
  "deprecated" boolean DEFAULT FALSE,
  "category_id" integer,
  "lowercase_name" varchar,
  "issues_url" varchar,
  "cookbook_followers_count" integer DEFAULT 0,
  "user_id" integer,
  "replacement_id" integer,
  "web_download_count" integer DEFAULT 0,
  "api_download_count" integer DEFAULT 0,
  "featured" boolean DEFAULT FALSE,
  "up_for_adoption" boolean,
  "privacy" boolean,
  "badges_mask" integer DEFAULT 0 NOT NULL,
  "deprecation_reason" varchar
);
CREATE UNIQUE INDEX "index_cookbooks_on_lowercase_name" ON "cookbooks" ("lowercase_name");
CREATE INDEX "index_cookbooks_on_name" ON "cookbooks" ("name");
CREATE INDEX "index_cookbooks_on_user_id" ON "cookbooks" ("user_id");
CREATE TABLE "email_preferences" (
  "id" serial PRIMARY KEY,
  "user_id" integer NOT NULL,
  "system_email_id" integer NOT NULL,
  "token" varchar NOT NULL,
  "created_at" timestamp,
  "updated_at" timestamp
);
CREATE UNIQUE INDEX "index_email_preferences_on_token" ON "email_preferences" ("token");
CREATE UNIQUE INDEX "index_email_preferences_on_user_id_and_system_email_id" ON "email_preferences" ("user_id", "system_email_id");
CREATE TABLE "group_members" (
  "id" serial PRIMARY KEY,
  "group_id" integer,
  "user_id" integer,
  "admin" boolean,
  "created_at" timestamp,
  "updated_at" timestamp
);
CREATE UNIQUE INDEX "index_group_members_on_user_id_and_group_id" ON "group_members" ("user_id", "group_id");
CREATE TABLE "group_resources" (
  "id" serial PRIMARY KEY,
  "group_id" integer,
  "resourceable_id" integer,
  "resourceable_type" varchar,
  "created_at" timestamp,
  "updated_at" timestamp
);
CREATE TABLE "groups" (
  "id" serial PRIMARY KEY,
  "name" varchar,
  "created_at" timestamp,
  "updated_at" timestamp
);
CREATE TABLE "hits" (
  "id" serial PRIMARY KEY,
  "label" varchar NOT NULL,
  "total" integer DEFAULT 0 NOT NULL
);
CREATE UNIQUE INDEX "index_hits_on_label" ON "hits" ("label");
CREATE TABLE "invitations" (
  "id" serial PRIMARY KEY,
  "organization_id" integer,
  "email" varchar,
  "token" varchar,
  "admin" boolean,
  "accepted" boolean,
  "created_at" timestamp,
  "updated_at" timestamp
);
CREATE INDEX "index_invitations_on_organization_id" ON "invitations" ("organization_id");
CREATE TABLE "metric_results" (
  "id" serial PRIMARY KEY,
  "cookbook_version_id" integer,
  "quality_metric_id" integer NOT NULL,
  "failure" boolean,
  "feedback" varchar,
  "created_at" timestamp NOT NULL,
  "updated_at" timestamp NOT NULL
);
CREATE TABLE "organizations" (
  "id" serial PRIMARY KEY,
  "created_at" timestamp,
  "updated_at" timestamp
);
CREATE TABLE "ownership_transfer_requests" (
  "id" serial PRIMARY KEY,
  "cookbook_id" integer NOT NULL,
  "recipient_id" integer NOT NULL,
  "sender_id" integer NOT NULL,
  "token" varchar NOT NULL,
  "accepted" boolean,
  "created_at" timestamp,
  "updated_at" timestamp,
  "add_owner_as_collaborator" boolean
);
CREATE INDEX "index_ownership_transfer_requests_on_cookbook_id" ON "ownership_transfer_requests" ("cookbook_id");
CREATE INDEX "index_ownership_transfer_requests_on_recipient_id" ON "ownership_transfer_requests" ("recipient_id");
CREATE UNIQUE INDEX "index_ownership_transfer_requests_on_token" ON "ownership_transfer_requests" ("token");
CREATE TABLE "quality_metrics" (
  "id" serial PRIMARY KEY,
  "name" varchar,
  "created_at" timestamp NOT NULL,
  "updated_at" timestamp NOT NULL,
  "admin_only" boolean DEFAULT FALSE NOT NULL
);
CREATE UNIQUE INDEX "index_quality_metrics_on_name" ON "quality_metrics" ("name");
CREATE TABLE "supported_platforms" (
  "id" serial PRIMARY KEY,
  "name" varchar NOT NULL,
  "version_constraint" varchar DEFAULT '>= 0.0.0' NOT NULL,
  "created_at" timestamp,
  "updated_at" timestamp
);
CREATE UNIQUE INDEX "index_supported_platforms_on_name_and_version_constraint" ON "supported_platforms" ("name", "version_constraint");
CREATE TABLE "system_emails" (
  "id" serial PRIMARY KEY,
  "name" varchar NOT NULL,
  "created_at" timestamp,
  "updated_at" timestamp
);
CREATE UNIQUE INDEX "index_system_emails_on_name" ON "system_emails" ("name");
CREATE TABLE "tools" (
  "id" serial PRIMARY KEY,
  "user_id" integer,
  "name" varchar,
  "type" varchar,
  "description" text,
  "source_url" varchar,
  "instructions" text,
  "created_at" timestamp,
  "updated_at" timestamp,
  "lowercase_name" varchar,
  "slug" varchar,
  "up_for_adoption" boolean
);
CREATE UNIQUE INDEX "index_tools_on_lowercase_name" ON "tools" ("lowercase_name");
CREATE UNIQUE INDEX "index_tools_on_slug" ON "tools" ("slug");
CREATE INDEX "index_tools_on_user_id" ON "tools" ("user_id");
CREATE TABLE "users" (
  "id" serial PRIMARY KEY,
  "first_name" varchar,
  "last_name" varchar,
  "created_at" timestamp,
  "updated_at" timestamp,
  "company" varchar,
  "roles_mask" integer,
  "email" varchar DEFAULT '' NOT NULL,
  "jira_username" varchar,
  "twitter_username" varchar,
  "public_key" text,
  "install_preference" varchar,
  "slack_username" varchar
);
