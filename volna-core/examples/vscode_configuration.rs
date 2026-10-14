fn main() {
    println!(
        "{}",
        serde_json::to_string_pretty(&volna_core::settings::schema::vscode_configuration())
            .unwrap()
    );
}
