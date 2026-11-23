require "rails_helper"
require "tmpdir"

describe CookstyleHelpers do
  let(:artifact_dir) { Dir.mktmpdir("cookstyle-helpers") }

  before do
    FileUtils.mkdir_p(File.join(artifact_dir, "recipes"))
    File.write(File.join(artifact_dir, "recipes", "default.rb"), "node.set['x'] = 1\n")
  end

  after { FileUtils.remove_entry(artifact_dir) }

  describe ".process_artifact" do
    it "returns the offenses and the failure status" do
      feedback, status = described_class.process_artifact(artifact_dir)

      expect(feedback).to match(%r{Chef/Deprecations/NodeSet})
      expect(status).to be(true)
    end

    it "uses the cookstyle configuration rather than the artifact's .rubocop.yml" do
      File.write(File.join(artifact_dir, ".rubocop.yml"), "AllCops: [\n")

      expect { described_class.process_artifact(artifact_dir) }.to_not raise_error
    end
  end

  describe ".cookstyle_command" do
    it "passes the arguments without shell quoting" do
      command = described_class.cookstyle_command(artifact_dir)

      expect(command.first).to eq("cookstyle")
      expect(command).to include(artifact_dir)
      expect(command).to include("--config", described_class.cookstyle_config)
    end
  end
end
