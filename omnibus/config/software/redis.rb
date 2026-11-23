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

# Project copy of omnibus-software's redis definition. Since Redis 8 the
# project ships under the tri-license RSALv2/SSPLv1/AGPLv3 and puts the terms
# in LICENSE.txt, so the definition from omnibus-software (still on 7.0.8 and
# the old BSD-3-Clause notice) no longer matches the tarball.

name 'redis'

license 'AGPL-3.0-only'
license_file 'LICENSE.txt'
skip_transitive_dependency_licensing true

dependency 'config_guess'

default_version '8.12.0'

version('8.12.0') { source sha256: 'b0100a879b2d45dc97d8fda941127609d648f8913008db1b65a2f72a8ae36c8b' }

source url: "https://download.redis.io/releases/redis-#{version}.tar.gz"
internal_source url: "#{ENV['ARTIFACTORY_REPO_URL']}/#{name}/#{name}-#{version}.tar.gz",
                authorization: "X-JFrog-Art-Api:#{ENV['ARTIFACTORY_TOKEN']}"

relative_path "redis-#{version}"

build do
  env = with_standard_compiler_flags(with_embedded_path).merge(
    'PREFIX' => "#{install_dir}/embedded"
  )

  update_config_guess

  make "-j #{workers}", env: env
  make 'install', env: env
end
