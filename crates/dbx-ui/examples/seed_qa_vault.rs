//! Provision only the repository's disposable QA vault using DBX's real codec.
#[allow(dead_code)]
#[path = "../src/vault.rs"]
mod vault;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let directory =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/ui-test/config/dbx");
    let vault = vault::CredentialVault::at(directory.join("credentials.vault"));
    match vault.state() {
        vault::VaultState::Uninitialized => vault.create("dbx-ui-test-vault")?,
        vault::VaultState::Locked => vault.unlock("dbx-ui-test-vault")?,
        vault::VaultState::Unlocked => {}
    }
    let profiles: serde_json::Value =
        serde_json::from_slice(&std::fs::read(directory.join("connections.json"))?)?;
    for profile in profiles["connections"]
        .as_array()
        .ok_or("missing QA profiles")?
    {
        let is_fixture = matches!(
            (profile["kind"].as_str(), profile["url"].as_str()),
            (
                Some("postgresql"),
                Some("postgres://dbx_test@127.0.0.1:55432/dbx_test")
            ) | (
                Some("mysql"),
                Some("mysql://dbx_test@127.0.0.1:53306/dbx_test")
            )
        );
        if is_fixture
            && let Some(key) = profile["secret_key"].as_str()
            && vault.get(key)?.is_none()
        {
            vault.set(key, "dbx_test_password")?;
        }
    }
    println!("Disposable QA vault ready.");
    Ok(())
}
