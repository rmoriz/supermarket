//! Prints the route manifest as TSV: verb, path, action. `harness/routes.tsv` is a
//! committed copy so reviewers can diff routing against `config/routes.rb`.

fn main() {
    for route in supermarket_rs::routes::table::ROUTES {
        println!("{}\t{}\t{}", route.method, route.path, route.action);
    }
}
