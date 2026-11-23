#
# Copyright:: Chef Software, Inc.
#
# Licensed under the Apache License, Version 2.0 (the "License");
# you may not use this file except in compliance with the License.
# You may obtain a copy of the License at
#
#     http://www.apache.org/licenses/LICENSE-2.0
#
# Unless required by applicable law or agreed to in writing, software
# distributed under the License is distributed on an "AS IS" BASIS,
# WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
# See the License for the specific language governing permissions and
# limitations under the License.
#

# PostgreSQL 17 is the running cluster of the package. The postgresql13
# definition stays around and is still built, because the pg_upgrade resource
# needs the binaries of a 13 installation to migrate an existing cluster.
#
# Both definitions install their binaries below embedded/postgresql/<major>
# and the later build links them to embedded/bin, which is what the runit
# service and the enterprise_pg_cluster resource use. postgresql13 therefore
# has to be listed before postgresql17 in config/projects/supermarket.rb.

name 'postgresql17'

default_version '17.12'

license 'PostgreSQL'
license_file 'COPYRIGHT'
skip_transitive_dependency_licensing true

dependency 'zlib'
dependency 'openssl'
dependency 'libedit'
dependency 'ncurses'
dependency 'libossp-uuid'
dependency 'config_guess'

source url: "https://ftp.postgresql.org/pub/source/v#{version}/postgresql-#{version}.tar.bz2"

version('17.12') { source sha256: '9cf136a703d8ee2f83465c43be4e9e8e7930c82b0bfba13be7e2537be769ac55' }

relative_path "postgresql-#{version}"

build do
  env = with_standard_compiler_flags(with_embedded_path)
  short_version = version.match(/^([0-9]+)/).to_s

  update_config_guess(target: 'config')

  command './configure' \
          " --prefix=#{install_dir}/embedded/postgresql/#{short_version}" \
          ' --with-libedit-preferred' \
          ' --with-openssl' \
          ' --with-ossp-uuid' \
          " --with-includes=#{install_dir}/embedded/include" \
          " --with-libraries=#{install_dir}/embedded/lib", env: env

  make "world -j #{workers}", env: env
  make "install-world -j #{workers}", env: env

  block do
    Dir.glob("#{install_dir}/embedded/postgresql/#{short_version}/bin/*").sort.each do |bin|
      link bin, "#{install_dir}/embedded/bin/#{File.basename(bin)}"
    end

    delete "#{install_dir}/embedded/postgresql/#{short_version}/share/doc"
  end
end
