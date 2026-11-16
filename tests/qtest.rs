#[test]
fn t() {
    println!(
        "{:?}",
        serde_urlencoded::from_str::<Vec<(String, String)>>(
            "q=x&badges[]=a&badges[]=b&platforms[]=d"
        )
    );
    println!(
        "{:?}",
        serde_urlencoded::from_str::<Vec<(String, String)>>("")
    );
}
