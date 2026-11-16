#!/usr/bin/env ruby
# Generates Rails cookie-crypto vectors using the same primitives ActionDispatch uses
# when config.action_dispatch.use_authenticated_cookie_encryption = true
# (Rails >= 5.2 default, in force here through load_defaults 7.1).
#
# Usage: ruby harness/cookie_vectors.rb > harness/fixtures/cookies.json
gem "json"
require "json"
require_relative "normalize"
require "active_support"
require "active_support/key_generator"
require "active_support/message_encryptor"
require "openssl"

SECRET = "a" * 128
COOKIE = "_supermarket_session"
SALT = "authenticated encrypted cookie"
PURPOSE = "cookie.#{COOKIE}"

def encryptor
  key = ActiveSupport::KeyGenerator.new(SECRET, iterations: 1000, hash_digest_class: OpenSSL::Digest::SHA256).generate_key(SALT, 32)
  ActiveSupport::MessageEncryptor.new(key, cipher: "aes-256-gcm", serializer: JSON)
end

sessions = [
  { "user_id" => 7, "csrf" => "abc123", "session_id" => "s1", "return_to" => nil },
  { "csrf" => "zzz", "session_id" => "s2" },
  { "user_id" => nil, "csrf" => "", "session_id" => "s3", "return_to" => "/cookbooks?page=2" },
  { "user_id" => 42, "csrf" => "q" * 64, "session_id" => "s4", "flash" => { "notice" => "hi – ünïcode" } },
]

e = encryptor
vectors = sessions.map do |session|
  cookie = e.encrypt_and_sign(session, purpose: PURPOSE)
  readback = encryptor.decrypt_and_verify(cookie, purpose: PURPOSE)
  { session: session, cookie: cookie, readback: readback }
end

good = vectors.first[:cookie]
negatives = [
  { name: "wrong-secret" },
  { name: "no-purpose" },
  { name: "tampered-tag", cookie: good.sub(/(.*)--(.*)--/, '\1--\2--zzzz') },
  { name: "truncated", cookie: good[0..(good.length / 2)] },
  { name: "garbage", cookie: "not-a-cookie" },
]
negatives.each do |n|
  n[:cookie] ||= encryptor.encrypt_and_sign(vectors.first[:session], purpose: "cookie.other")
  begin
    key = n[:name] == "wrong-secret" ? ActiveSupport::KeyGenerator.new("b" * 64, iterations: 1000, hash_digest_class: OpenSSL::Digest::SHA256).generate_key(SALT, 32) : nil
    enc = key ? ActiveSupport::MessageEncryptor.new(key, cipher: "aes-256-gcm", serializer: JSON) : encryptor
    purpose = n[:name] == "no-purpose" ? nil : PURPOSE
    n[:readback] = enc.decrypt_and_verify(n[:cookie], purpose: purpose)
  rescue StandardError => err
    n[:readback] = nil
    n[:error] = err.class.name
  end
end

puts JSON.pretty_generate({ secret_key_base: SECRET, cookie_name: COOKIE, purpose: PURPOSE, vectors: vectors, negatives: negatives })
