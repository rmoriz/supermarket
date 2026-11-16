#!/usr/bin/env ruby
# Run: ruby harness/spec/normalize_test.rb
require "json"
require_relative "../normalize"

$failures = 0

def check(name)
  ok = yield
  puts(ok ? "ok   #{name}" : "FAIL #{name}")
  $failures += 1 unless ok
end

def json_body(hash) = [JSON.dump(hash), { "content-type" => "application/json" }]

a, ha = json_body({ "created_at" => "2024-01-01T10:00:00.000Z", "name" => "redis" })
b, hb = json_body({ "created_at" => "2025-05-05T11:11:11.000Z", "name" => "redis" })
c, hc = json_body({ "created_at" => "2024-01-01T10:00:00.000Z", "name" => "redisio" })

check("timestamps normalize equal") do
  Normalize.response(status: 200, headers: ha, raw_body: a) ==
    Normalize.response(status: 200, headers: hb, raw_body: b)
end

check("a different name still differs") do
  Normalize.response(status: 200, headers: ha, raw_body: a) !=
    Normalize.response(status: 200, headers: hc, raw_body: c)
end

check("the brand swap normalizes") do
  Normalize.response(status: 200, headers: { "content-type" => "text/html" }, raw_body: "<h1>Chef Supermarket</h1>") ==
    Normalize.response(status: 200, headers: { "content-type" => "text/html" }, raw_body: "<h1>Supermarket</h1>")
end

check("html attribute order normalizes") do
  Normalize.response(status: 200, headers: { "content-type" => "text/html" }, raw_body: '<a href="/x" class="y">z</a>') ==
    Normalize.response(status: 200, headers: { "content-type" => "text/html" }, raw_body: '<a class="y" href="/x">z</a>')
end

check("cookie ciphertext normalizes but the name does not") do
  one = Normalize.response(status: 200, headers: { "set-cookie" => "_supermarket_session=aaa--bbb--ccc; path=/" }, raw_body: "")
  two = Normalize.response(status: 200, headers: { "set-cookie" => "_supermarket_session=ddd--eee--fff; path=/" }, raw_body: "")
  three = Normalize.response(status: 200, headers: { "set-cookie" => "_other=ddd; path=/" }, raw_body: "")
  one == two && one != three
end

check("live redis counters normalize, other redis fields do not") do
  one = Normalize.response(status: 200, headers: ha, raw_body: JSON.dump({ "redis" => { "connected_clients" => 3, "used_memory_rss" => 1 } }))
  two = Normalize.response(status: 200, headers: ha, raw_body: JSON.dump({ "redis" => { "connected_clients" => 7, "used_memory_rss" => 1 } }))
  three = Normalize.response(status: 200, headers: ha, raw_body: JSON.dump({ "redis" => { "connected_clients" => 7, "used_memory_rss" => 2 } }))
  one == two && one != three
end

check("status differences survive") do
  Normalize.response(status: 200, headers: {}, raw_body: "x") !=
    Normalize.response(status: 404, headers: {}, raw_body: "x")
end

check("the port-only chat feature key is dropped from a features map") do
  rails, = json_body({ "features" => { "tools" => true } })
  rust, = json_body({ "features" => { "tools" => true, "chat" => false } })
  Normalize.response(status: 200, headers: ha, raw_body: rails) ==
    Normalize.response(status: 200, headers: ha, raw_body: rust)
end

check("chat outside a features map still differs") do
  one, = json_body({ "channels" => { "chat" => 1 } })
  two, = json_body({ "channels" => { "chat" => 2 } })
  Normalize.response(status: 200, headers: ha, raw_body: one) !=
    Normalize.response(status: 200, headers: ha, raw_body: two)
end

puts($failures.zero? ? "all normalize tests passed" : "#{$failures} failure(s)")
exit($failures.zero? ? 0 : 1)
