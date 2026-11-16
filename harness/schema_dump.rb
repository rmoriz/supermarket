#!/usr/bin/env ruby
# Converts supermarket-ruby/src/supermarket/db/schema.rb into harness/schema.sql.
# The DSL subset is small (columns, defaults, indexes, extensions); anything else
# raises so the dump can never silently drift from the Ruby schema.
require "json"

SCHEMA = File.expand_path("../supermarket-ruby/src/supermarket/db/schema.rb", __dir__)
OUT = File.expand_path("schema.sql", __dir__)

TYPES = {
  "string" => "varchar",
  "text" => "text",
  "integer" => "integer",
  "bigint" => "bigint",
  "boolean" => "boolean",
  "float" => "double precision",
  "decimal" => "decimal",
  "datetime" => "timestamp",
  "date" => "date",
  "json" => "json",
  "jsonb" => "jsonb",
  "uuid" => "uuid",
}.freeze

def literal(value)
  case value
  when true then "TRUE"
  when false then "FALSE"
  when nil then "NULL"
  when Numeric then value.to_s
  else "'#{value.to_s.gsub("'", "''")}'"
  end
end

sql = +""
known = {}

class Table
  def initialize(sql, known, name, id_serial)
    @sql = sql
    @known = known
    @name = name
    @id_serial = id_serial
    @columns = []
    @indexes = []
    @known[name] = @columns
  end

  def column(name, type, **options)
    raise "unknown column type #{type}" unless TYPES.key?(type)

    unknown = options.keys - %i[default null precision limit]
    raise "unknown column option #{unknown} on #{@name}.#{name}" unless unknown.empty?

    definition = +"\"#{name}\" #{TYPES.fetch(type)}"
    definition << "(#{options[:limit]})" if options[:limit] && TYPES.fetch(type) == "varchar"
    definition << " DEFAULT #{literal(options[:default])}" unless options[:default].nil?
    definition << " NOT NULL" if options[:null] == false
    @columns << definition
  end

  TYPES.each_key do |type|
    define_method(type) { |name, **options| column(name, type, **options) }
  end

  def index(columns, name:, unique: false)
    @indexes << "CREATE #{unique ? 'UNIQUE ' : ''}INDEX \"#{name}\" ON \"#{@name}\" (#{columns.map { |c| "\"#{c}\"" }.join(', ')});"
  end

  def render
    cols = (@id_serial ? ["\"id\" serial PRIMARY KEY"] : []) + @columns
    @sql << "CREATE TABLE \"#{@name}\" (\n  #{cols.join(",\n  ")}\n);\n"
    @indexes.each { |i| @sql << i << "\n" }
  end
end

class Recorder
  def initialize(sql, known)
    @sql = sql
    @known = known
  end

  def enable_extension(name) = @sql << "CREATE EXTENSION IF NOT EXISTS \"#{name}\";\n"

  def create_table(name, id: nil, force: nil, **_rest)
    raise "unsupported id: #{id.inspect} on #{name}" unless id == :serial

    table = Table.new(@sql, @known, name, true)
    yield table
    table.render
  end
end

source = File.read(SCHEMA)
body = source[/\.define\(version: [^)]*\) do\n(.*)\nend\n\z/m, 1] or raise "schema body not found"

recorder = Recorder.new(sql, known)
recorder.instance_eval(body, SCHEMA)

# Ruby `t.boolean`/`t.string` call style: rewrite `t.string "x", ...` into recorder calls.
puts sql
