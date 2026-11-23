require "mixlib/shellout"
require "dotenv-rails"
Dotenv.load(".env")

module CookstyleHelpers
  def self.process_artifact(path)
    shell_out = Mixlib::ShellOut.new(*cookstyle_command(path))
    shell_out.run_command
    parse_cookstyle_output(shell_out.stdout)
  rescue StandardError => e
    raise "Error in processing Artifact #{e.message}"
  end

  # Build the cookstyle invocation. The configuration is pinned to the one
  # shipped with cookstyle itself: without an explicit --config, RuboCop also
  # picks up the .rubocop.yml found inside the artifact, which is evaluated as
  # ERB and resolves require/inherit_from paths from there.
  def self.cookstyle_command(path)
    command = ["cookstyle", path, "--config", cookstyle_config, "--format", "json"]
    command.push("--only", ENV["COOKSTYLE_COPS"]) if ENV["COOKSTYLE_COPS"].to_s != ""
    command
  end

  def self.cookstyle_config
    binary = Gem.bin_path("cookstyle", "cookstyle")
    File.expand_path("../config/default.yml", File.dirname(binary))
  end

  # "COP_NAME":"MESSAGE":"file_name:line=line_no,col=col_no"
  def self.parse_cookstyle_output(cookstyle_output)
    status = ""
    offenses_arr = JSON.parse(cookstyle_output)["files"].map { |h|
      h["offenses"]\
        .each { |a| a.merge!("file" => h["path"]) }
    }.flatten.sort_by { |hsh| hsh["cop_name"] }\
      .each { |a|
        status << "#{[a["cop_name"], a["message"],\
        a["file"], a["location"]["line"]].join(": ")}\n"
      }

    [status, offenses_arr.size > 0]
  end
end
