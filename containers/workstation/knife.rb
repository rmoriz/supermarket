node_name 'workstation'
client_key '/keys/workstation.pem'
chef_server_url 'http://microwave:8000/organizations/demo'
supermarket_site 'http://supermarket:3000'
knife[:supermarket_site] = 'http://supermarket:3000'
syntax_check_cache_path '/tmp/knife-syntaxcache'
ssl_verify_mode :verify_none
