require "spec_helper"

describe Supermarket::RedisUrl do
  describe ".normalize" do
    it "drops the namespace suffix" do
      expect(described_class.normalize("redis://localhost:6379/0/supermarket"))
        .to eq("redis://localhost:6379/0")
    end

    it "leaves URLs without a namespace untouched" do
      expect(described_class.normalize("redis://localhost:6379/2"))
        .to eq("redis://localhost:6379/2")
    end

    it "leaves URLs without a database untouched" do
      expect(described_class.normalize("redis://redis.example.com:6379"))
        .to eq("redis://redis.example.com:6379")
    end
  end
end
