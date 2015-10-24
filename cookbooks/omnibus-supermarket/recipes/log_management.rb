#
# Cookbook Name:: supermarket
# Recipe:: log_management
#
# Copyright 2015 Chef Software, Inc.
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

# Omnibus Supermarket log management is mostly handled by runit, svlogd
# and each component service logged to STDOUT for svlogd to handle. Some
# components do not handle logging to STDOUT well, so logrotate is added
# to manage the logs generated by those services.

include_recipe 'omnibus-supermarket::config'

directory "#{node['supermarket']['var_directory']}/etc/logrotate.d" do
  owner 'root'
  group 'root'
  mode '0644'
end

template "#{node['supermarket']['var_directory']}/etc/logrotate.conf" do
  source "logrotate.conf.erb"
  mode   "0644"
  owner  "root"
  group  "root"
  variables(
    var_directory: node['supermarket']['var_directory'],
  )
end

template "/etc/cron.hourly/supermarket_logrotate" do
  source "logrotate.cron.erb"
  mode   "0755"
  owner  "root"
  group  "root"
  variables(
    install_directory: node['supermarket']['install_directory'],
    var_directory: node['supermarket']['var_directory'],
  )
end
