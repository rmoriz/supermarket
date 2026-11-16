//! Every route of the reference application, in `config/routes.rb` order.
//! `harness/routes.tsv` is the reviewable copy; `tests` asserts parity both ways.

pub struct Route {
    pub method: &'static str,
    pub path: &'static str,
    pub action: &'static str,
}

macro_rules! r {
    ($method:literal, $path:literal, $action:literal) => {
        Route {
            method: $method,
            path: $path,
            action: $action,
        }
    };
}

pub static ROUTES: &[Route] = &[
    // development-only LetterOpenerWeb mount is not ported (dev tool).
    r!("POST", "/fieri/jobs", "fieri/jobs#create"),
    r!("GET", "/fieri/status", "fieri/status#show"),
    // api/v1
    r!("GET", "/api/v1/metrics", "api/v1/metrics#show"),
    r!("GET", "/api/v1/health", "api/v1/health#show"),
    r!("GET", "/api/v1/cookbooks", "api/v1/cookbooks#index"),
    r!("GET", "/api/v1/search", "api/v1/cookbooks#search"),
    r!(
        "GET",
        "/api/v1/cookbooks/{cookbook}",
        "api/v1/cookbooks#show"
    ),
    r!(
        "GET",
        "/api/v1/cookbooks/{cookbook}/contingent",
        "api/v1/cookbooks#contingent"
    ),
    r!(
        "GET",
        "/api/v1/cookbooks/{cookbook}/versions/{version}",
        "api/v1/cookbook_versions#show"
    ),
    r!(
        "GET",
        "/api/v1/cookbooks/{cookbook}/versions/{version}/download",
        "api/v1/cookbook_versions#download"
    ),
    r!(
        "POST",
        "/api/v1/cookbooks",
        "api/v1/cookbook_uploads#create"
    ),
    r!(
        "DELETE",
        "/api/v1/cookbooks/{cookbook}",
        "api/v1/cookbook_uploads#destroy"
    ),
    r!(
        "DELETE",
        "/api/v1/cookbooks/{cookbook}/versions/{version}",
        "api/v1/cookbook_uploads#destroy_version"
    ),
    r!("GET", "/api/v1/users/{user}", "api/v1/users#show"),
    r!(
        "POST",
        "/api/v1/cookbook-versions/cookstyle_evaluation",
        "api/v1/quality_metrics#cookstyle_evaluation"
    ),
    r!(
        "POST",
        "/api/v1/cookbook-versions/collaborators_evaluation",
        "api/v1/quality_metrics#collaborators_evaluation"
    ),
    r!(
        "POST",
        "/api/v1/cookbook-versions/publish_evaluation",
        "api/v1/quality_metrics#publish_evaluation"
    ),
    r!(
        "POST",
        "/api/v1/quality_metrics/cookstyle_evaluation",
        "api/v1/quality_metrics#cookstyle_evaluation"
    ),
    r!(
        "POST",
        "/api/v1/quality_metrics/collaborators_evaluation",
        "api/v1/quality_metrics#collaborators_evaluation"
    ),
    r!(
        "POST",
        "/api/v1/quality_metrics/publish_evaluation",
        "api/v1/quality_metrics#publish_evaluation"
    ),
    r!(
        "POST",
        "/api/v1/quality_metrics/supported_platforms_evaluation",
        "api/v1/quality_metrics#supported_platforms_evaluation"
    ),
    r!(
        "POST",
        "/api/v1/quality_metrics/contributing_file_evaluation",
        "api/v1/quality_metrics#contributing_file_evaluation"
    ),
    r!(
        "POST",
        "/api/v1/quality_metrics/testing_file_evaluation",
        "api/v1/quality_metrics#testing_file_evaluation"
    ),
    r!(
        "POST",
        "/api/v1/quality_metrics/version_tag_evaluation",
        "api/v1/quality_metrics#version_tag_evaluation"
    ),
    r!(
        "POST",
        "/api/v1/quality_metrics/no_binaries_evaluation",
        "api/v1/quality_metrics#no_binaries_evaluation"
    ),
    r!("GET", "/api/v1/tools/{tool}", "api/v1/tools#show"),
    r!("GET", "/api/v1/tools", "api/v1/tools#index"),
    r!("GET", "/api/v1/tools-search", "api/v1/tools#search"),
    // web
    r!("GET", "/cookbooks-directory", "cookbooks#directory"),
    r!(
        "GET",
        "/available_for_adoption",
        "cookbooks#available_for_adoption"
    ),
    r!("GET", "/universe", "api/v1/universe#index"),
    r!("GET", "/api/v1/universe", "api/v1/universe#index"),
    r!("GET", "/status", "api/v1/health#show"),
    r!(
        "GET",
        "/unsubscribe/{token}",
        "email_preferences#unsubscribe"
    ),
    r!(
        "PUT",
        "/cookbooks/{id}/transfer_ownership",
        "transfer_ownership#transfer"
    ),
    r!(
        "GET",
        "/ownership_transfer/{token}/accept",
        "transfer_ownership#accept"
    ),
    r!(
        "GET",
        "/ownership_transfer/{token}/decline",
        "transfer_ownership#decline"
    ),
    r!("GET", "/cookbooks", "cookbooks#index"),
    r!("GET", "/cookbooks/{id}", "cookbooks#show"),
    r!("PATCH", "/cookbooks/{id}", "cookbooks#update"),
    r!("PUT", "/cookbooks/{id}", "cookbooks#update"),
    r!("GET", "/cookbooks/{id}/download", "cookbooks#download"),
    r!("PUT", "/cookbooks/{id}/follow", "cookbooks#follow"),
    r!("DELETE", "/cookbooks/{id}/unfollow", "cookbooks#unfollow"),
    r!("PUT", "/cookbooks/{id}/deprecate", "cookbooks#deprecate"),
    r!(
        "DELETE",
        "/cookbooks/{id}/deprecate",
        "cookbooks#undeprecate"
    ),
    r!(
        "PUT",
        "/cookbooks/{id}/toggle_featured",
        "cookbooks#toggle_featured"
    ),
    r!(
        "GET",
        "/cookbooks/{id}/deprecate_search",
        "cookbooks#deprecate_search"
    ),
    r!("POST", "/cookbooks/{id}/adoption", "cookbooks#adoption"),
    r!(
        "GET",
        "/cookbooks/{id}/versions/{version}/download",
        "cookbook_versions#download"
    ),
    r!(
        "GET",
        "/cookbooks/{id}/versions/{version}",
        "cookbook_versions#show"
    ),
    r!("GET", "/collaborators", "collaborators#index"),
    r!("GET", "/collaborators/new", "collaborators#new"),
    r!("POST", "/collaborators", "collaborators#create"),
    r!(
        "PUT",
        "/collaborators/{id}/transfer",
        "collaborators#transfer"
    ),
    r!(
        "DELETE",
        "/collaborators/{id}/destroy_group",
        "collaborators#destroy_group"
    ),
    r!("DELETE", "/collaborators/{id}", "collaborators#destroy"),
    r!("GET", "/users/{id}", "users#show"),
    r!("GET", "/users/{id}/tools", "users#tools"),
    r!("GET", "/users/{id}/groups", "users#groups"),
    r!("PUT", "/users/{id}/make_admin", "users#make_admin"),
    r!("DELETE", "/users/{id}/revoke_admin", "users#revoke_admin"),
    r!(
        "GET",
        "/users/{id}/followed_cookbook_activity",
        "users#followed_cookbook_activity"
    ),
    r!(
        "DELETE",
        "/users/{user_id}/accounts/{id}",
        "accounts#destroy"
    ),
    r!("GET", "/groups", "groups#index"),
    r!("GET", "/groups/new", "groups#new"),
    r!("POST", "/groups", "groups#create"),
    r!("GET", "/groups/{id}", "groups#show"),
    r!("GET", "/groups/{id}/edit", "groups#edit"),
    r!("PATCH", "/groups/{id}", "groups#update"),
    r!("PUT", "/groups/{id}", "groups#update"),
    r!("DELETE", "/groups/{id}", "groups#destroy"),
    r!("GET", "/group_members", "group_members#index"),
    r!("GET", "/group_members/new", "group_members#new"),
    r!("POST", "/group_members", "group_members#create"),
    r!("GET", "/group_members/{id}", "group_members#show"),
    r!("GET", "/group_members/{id}/edit", "group_members#edit"),
    r!("PATCH", "/group_members/{id}", "group_members#update"),
    r!("PUT", "/group_members/{id}", "group_members#update"),
    r!("DELETE", "/group_members/{id}", "group_members#destroy"),
    r!(
        "POST",
        "/group_members/{id}/make_admin",
        "group_members#make_admin"
    ),
    r!("GET", "/tools", "tools#index"),
    r!("GET", "/tools/new", "tools#new"),
    r!("POST", "/tools", "tools#create"),
    r!("GET", "/tools/{id}", "tools#show"),
    r!("GET", "/tools/{id}/edit", "tools#edit"),
    r!("PATCH", "/tools/{id}", "tools#update"),
    r!("PUT", "/tools/{id}", "tools#update"),
    r!("DELETE", "/tools/{id}", "tools#destroy"),
    r!("POST", "/tools/{id}/adoption", "tools#adoption"),
    r!("GET", "/tools-directory", "tools#directory"),
    r!("PATCH", "/profile", "profile#update"),
    r!("PUT", "/profile", "profile#update"),
    r!("GET", "/profile/edit", "profile#edit"),
    r!(
        "POST",
        "/profile/update_install_preference",
        "profile#update_install_preference"
    ),
    r!(
        "PATCH",
        "/profile/change_password",
        "profile#change_password"
    ),
    r!("GET", "/profile/link-github", "profile#link_github"),
    r!("GET", "/chat", "irc_logs#index"),
    r!("GET", "/chat/{channel}", "irc_logs#show"),
    r!("GET", "/chat/{channel}/{date}", "irc_logs#show"),
    r!("GET", "/auth/chef_oauth2/callback", "sessions#create"),
    r!("POST", "/auth/chef_oauth2/callback", "sessions#create"),
    r!("POST", "/auth/chef_oauth2", "sessions#passthru"),
    r!("GET", "/auth/failure", "sessions#failure"),
    r!("GET", "/login", "redirect#sign_in"),
    r!("GET", "/signin", "redirect#sign_in"),
    r!("GET", "/sign-in", "sessions#new"),
    r!("GET", "/sign-up", "sessions#new"),
    r!("DELETE", "/logout", "redirect#sign_out"),
    r!("DELETE", "/signout", "redirect#sign_out"),
    r!("DELETE", "/sign-out", "sessions#destroy"),
    r!("GET", "/auth/{provider}/callback", "accounts#create"),
    r!("POST", "/auth/{provider}/callback", "accounts#create"),
    r!("POST", "/auth/{provider}", "sessions#passthru"),
    r!("GET", "/dashboard", "pages#dashboard"),
    r!("GET", "/documentation", "pages#documentation"),
    r!("GET", "/robots", "pages#robots"),
    r!("GET", "/robots.txt", "pages#robots"),
    r!("GET", "/", "pages#welcome"),
];

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn upstream_literal_routes_are_all_present() {
        let source = std::fs::read_to_string("supermarket-ruby/src/supermarket/config/routes.rb")
            .expect("routes.rb");
        let re = regex::Regex::new(
            r#"^(\s*)(get|post|put|patch|delete|match|root)\s+"([^"]+)"(?:\s*=>\s*"([^"]+)")?"#,
        )
        .unwrap();
        let ns_re = regex::Regex::new(r"^\s*namespace :(\w+)").unwrap();

        // Vec, not set: `resources` blocks contribute a prefix the flat line does not
        // carry, so lookups below match on path suffix.
        let mut ours: Vec<(String, String, String)> = ROUTES
            .iter()
            .map(|r| {
                (
                    r.method.to_string(),
                    r.path.replace('{', ":").replace('}', ""),
                    r.action.to_string(),
                )
            })
            .collect();

        // (indent, name) so nested namespaces can be popped on dedent.
        let mut namespaces: Vec<(usize, String)> = Vec::new();
        let mut checked = 0;

        for line in source.lines() {
            if line.trim().is_empty() {
                continue;
            }
            let indent = line.len() - line.trim_start().len();
            // A line only belongs to namespaces opened at a shallower indent.
            while namespaces.last().is_some_and(|(depth, _)| *depth >= indent) {
                namespaces.pop();
            }

            if let Some(caps) = ns_re.captures(line) {
                namespaces.push((indent, caps[1].to_owned()));
                continue;
            }

            let Some(caps) = re.captures(line) else {
                continue;
            };
            let verb = caps[2].to_owned();
            let action = match caps.get(4) {
                Some(a) => {
                    let mut action = String::new();
                    for (_, ns) in &namespaces {
                        action.push_str(ns);
                        action.push('/');
                    }
                    action.push_str(a.as_str());
                    action
                }
                None if verb == "root" => "pages#welcome".to_owned(),
                None => continue,
            };
            let path = if verb == "root" {
                "/".to_owned()
            } else {
                let mut path = String::from("/");
                for (_, ns) in &namespaces {
                    path.push_str(ns);
                    path.push('/');
                }
                path.push_str(caps[3].trim_start_matches('/'));
                // `robots.#{format}` is the only interpolated path.
                path = path.replace(".:format", "");
                path
            };

            let methods: Vec<&str> = match verb.as_str() {
                "match" => vec!["GET", "POST"],
                "root" => vec!["GET"],
                other => vec![other],
            };
            for method in methods {
                checked += 1;
                let method = method.to_uppercase();
                let found = ours.iter().position(|(m, p, a)| {
                    *m == method && *a == action && (p == &path || p.ends_with(&path))
                });
                match found {
                    Some(index) => {
                        ours.remove(index);
                    }
                    None => panic!("missing route {method} {path} -> {action}"),
                }
            }
        }

        assert!(checked >= 50, "parser saw only {checked} literal routes");
        for (method, path, action) in &ours {
            assert!(
                path.starts_with("/cookbooks")
                    || path.starts_with("/collaborators")
                    || path.starts_with("/users")
                    || path.starts_with("/groups")
                    || path.starts_with("/group_members")
                    || path.starts_with("/tools")
                    || path.starts_with("/profile")
                    || path.starts_with("/fieri")
                    || path.starts_with("/auth")
                    || path == "/api/v1/universe"
                    || path == "/universe"
                    || matches!(path.as_str(), "/login" | "/signin" | "/logout" | "/signout")
                    || path == "/robots.txt",
                "unexpected route {method} {path} -> {action}"
            );
        }
    }

    #[test]
    fn no_duplicate_route_entries() {
        let mut seen = HashSet::new();
        for route in ROUTES {
            assert!(
                seen.insert((route.method, route.path)),
                "duplicate {} {}",
                route.method,
                route.path
            );
        }
    }
}
