//! Registers the `craftcloud://` scheme with the system, where that is done at run time.
//!
//! - Linux and the BSDs: the desktop entry (`packaging/linux/io.github.jonasbove.craftcloud.desktop`) declares
//!   `MimeType=x-scheme-handler/craftcloud;`; installing it is enough.
//! - macOS: `CFBundleURLTypes` in the app bundle's `Info.plist`.
//! - Windows: `HKCU\Software\Classes\craftcloud`, written here each time the desktop client or
//!   `craftcloud daemon` starts (no admin rights needed), so it always points at the current
//!   executable, also after an update moved it.

/// Makes `craftcloud://` links start this executable (Windows); elsewhere packaging does it.
pub fn register() -> Result<(), String> {
    #[cfg(windows)]
    {
        use winreg::RegKey;
        use winreg::enums::HKEY_CURRENT_USER;
        let exe = std::env::current_exe().map_err(|e| e.to_string())?.display().to_string();
        let (key, _) = RegKey::predef(HKEY_CURRENT_USER).create_subkey(r"Software\Classes\craftcloud").map_err(|e| e.to_string())?;
        key.set_value("", &"URL:CraftCloud link".to_owned()).map_err(|e| e.to_string())?;
        key.set_value("URL Protocol", &String::new()).map_err(|e| e.to_string())?;
        let (icon, _) = key.create_subkey("DefaultIcon").map_err(|e| e.to_string())?;
        icon.set_value("", &format!("\"{exe}\",0")).map_err(|e| e.to_string())?;
        let (command, _) = key.create_subkey(r"shell\open\command").map_err(|e| e.to_string())?;
        command.set_value("", &format!("\"{exe}\" \"%1\"")).map_err(|e| e.to_string())?;
    }
    Ok(())
}
