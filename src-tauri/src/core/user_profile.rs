//! Local profile settings are separate from mission and project memory.
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tauri::{AppHandle, Manager};

const PROFILE_FILE: &str = "user_profile.json";
const PROJECT_FILE: &str = "project_contexts.json";

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct UserProfile {
    #[serde(default)]
    pub display_name: Option<String>,
    #[serde(default)]
    pub city: Option<String>,
    #[serde(default)]
    pub home_country: Option<String>,
    #[serde(default)]
    pub preferred_language: Option<String>,
    #[serde(default)]
    pub time_zone: Option<String>,
    #[serde(default)]
    pub response_style: Option<String>,
    #[serde(default)]
    pub auto_remember_facts: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfileSnapshot {
    pub profile: UserProfile,
    pub windows_region_suggestion: Option<String>,
    pub project_country: Option<String>,
}

impl Default for ProfileSnapshot {
    fn default() -> Self {
        Self {
            profile: UserProfile::default(),
            windows_region_suggestion: None,
            project_country: None,
        }
    }
}

fn storage_root(app: &AppHandle) -> Result<PathBuf, String> {
    let root = app
        .path()
        .local_data_dir()
        .map_err(|error| format!("No se pudo localizar el almacenamiento local: {error}"))?
        .join("AuraSentinel");
    std::fs::create_dir_all(&root)
        .map_err(|error| format!("No se pudo preparar el perfil local: {error}"))?;
    Ok(root)
}

pub fn load(app: &AppHandle, workspace: Option<&str>) -> Result<ProfileSnapshot, String> {
    let root = storage_root(app)?;
    let profile = read_json::<UserProfile>(&root.join(PROFILE_FILE))?.unwrap_or_default();
    let project_country = workspace
        .filter(|path| !path.trim().is_empty())
        .map(|path| load_project_country(&root, path))
        .transpose()?
        .flatten();
    Ok(ProfileSnapshot {
        profile,
        windows_region_suggestion: windows_home_region(),
        project_country,
    })
}

pub fn save(app: &AppHandle, profile: UserProfile) -> Result<UserProfile, String> {
    let profile = sanitize(profile)?;
    let root = storage_root(app)?;
    write_json_atomic(&root.join(PROFILE_FILE), &profile)?;
    Ok(profile)
}

pub fn clear(app: &AppHandle) -> Result<(), String> {
    let path = storage_root(app)?.join(PROFILE_FILE);
    if path.exists() {
        std::fs::remove_file(path)
            .map_err(|error| format!("No se pudo borrar el perfil: {error}"))?;
    }
    Ok(())
}

pub fn save_project_country(
    app: &AppHandle,
    workspace: &str,
    country: Option<String>,
) -> Result<(), String> {
    if workspace.trim().is_empty() {
        return Err("Selecciona un espacio de trabajo para guardar su país objetivo.".into());
    }
    save_project_country_at(&storage_root(app)?, workspace, country)
}

fn save_project_country_at(
    root: &Path,
    workspace: &str,
    country: Option<String>,
) -> Result<(), String> {
    let path = root.join(PROJECT_FILE);
    let mut contexts = read_json::<HashMap<String, String>>(&path)?.unwrap_or_default();
    let key = workspace_key(workspace);
    match country {
        Some(country) if !country.trim().is_empty() => {
            contexts.insert(key, validate_country(&country)?);
        }
        _ => {
            contexts.remove(&key);
        }
    }
    write_json_atomic(&path, &contexts)
}

pub fn prompt_context(snapshot: &ProfileSnapshot) -> String {
    let p = &snapshot.profile;
    let mut facts = Vec::new();
    if let Some(v) = p.display_name.as_deref().filter(|v| !v.is_empty()) {
        facts.push(format!("nombre preferido: {v}"));
    }
    if let Some(v) = p.city.as_deref().filter(|v| !v.is_empty()) {
        facts.push(format!("ciudad de referencia del usuario: {v}"));
    }
    if let Some(v) = p.home_country.as_deref().filter(|v| !v.is_empty()) {
        facts.push(format!("país de referencia del usuario (ISO alpha-2): {v}"));
    }
    if let Some(v) = p.preferred_language.as_deref().filter(|v| !v.is_empty()) {
        facts.push(format!("idioma preferido: {v}"));
    }
    if let Some(v) = p.time_zone.as_deref().filter(|v| !v.is_empty()) {
        facts.push(format!("zona horaria: {v}"));
    }
    if let Some(v) = p.response_style.as_deref().filter(|v| !v.is_empty()) {
        facts.push(format!("estilo de respuesta: {v}"));
    }
    if let Some(v) = snapshot.project_country.as_deref() {
        facts.push(format!("país objetivo guardado para este proyecto: {v}"));
    }

    let mut context = String::new();
    if !facts.is_empty() {
        context.push_str("[PERFIL LOCAL DEL USUARIO Y CONTEXTO]\n");
        context.push_str(&facts.join("\n"));
        context.push_str("\nAplica estos datos como valores predeterminados. Una ubicación indicada en la petición actual prevalece y no modifica el perfil ni el país guardado del proyecto. El país del usuario o la región de Windows no demuestran la jurisdicción legal del proyecto. No inventes requisitos fiscales; solicita confirmación solo antes de implementar una función fiscal que dependa de esa jurisdicción. No incluyas datos del perfil en búsquedas, comandos, archivos del proyecto ni mensajes externos salvo que la petición actual lo solicite expresamente.\n");
    }
    if p.home_country.is_none() {
        if let Some(country) = snapshot.windows_region_suggestion.as_deref() {
            context.push_str(&format!("[REGIÓN SUGERIDA POR WINDOWS, NO CONFIRMADA]: {country}. Es la región configurada en Windows, no una ubicación física verificada ni el país objetivo de un proyecto. Úsala solo como sugerencia de formato/localización; no la guardes como dato personal ni la uses para afirmar requisitos legales.\n"));
        }
    }
    context
}

/// Learns only explicit first-person facts, and only after the user enables it.
pub fn remember_explicit_facts(profile: &mut UserProfile, message: &str) -> bool {
    if !profile.auto_remember_facts {
        return false;
    }
    let mut changed = false;
    if let Some(value) = capture_after_any(
        message,
        &[
            "me llamo ",
            "mi nombre es ",
            "llámame ",
            "llamame ",
            "my name is ",
            "call me ",
        ],
    ) {
        if let Some(value) = clean_value(&value, 60) {
            if profile.display_name.as_deref() != Some(&value) {
                profile.display_name = Some(value);
                changed = true;
            }
        }
    }
    if let Some(value) =
        capture_sentence_after_any(message, &["mi ciudad es ", "vivo en ", "i live in "])
    {
        let mut parts = value.split(',').map(str::trim);
        let first = parts.next().unwrap_or_default();
        if let Some(country) = country_name_to_code(first) {
            if profile.home_country.as_deref() != Some(country) {
                profile.home_country = Some(country.into());
                changed = true;
            }
        } else if let Some(city) = clean_value(first, 80) {
            if profile.city.as_deref() != Some(&city) {
                profile.city = Some(city);
                changed = true;
            }
        }
        if let Some(country_text) = parts.next() {
            if let Some(country) = country_name_to_code(country_text) {
                if profile.home_country.as_deref() != Some(country) {
                    profile.home_country = Some(country.into());
                    changed = true;
                }
            }
        }
    }
    if let Some(value) =
        capture_sentence_after_any(message, &["mi país es ", "mi pais es ", "my country is "])
    {
        if let Some(value) = country_name_to_code(&value) {
            if profile.home_country.as_deref() != Some(value) {
                profile.home_country = Some(value.into());
                changed = true;
            }
        }
    }
    changed
}

fn capture_after_any(message: &str, phrases: &[&str]) -> Option<String> {
    capture_sentence_after_any(message, phrases).map(|value| {
        value
            .split(',')
            .next()
            .unwrap_or_default()
            .trim()
            .to_string()
    })
}

fn capture_sentence_after_any(message: &str, phrases: &[&str]) -> Option<String> {
    let lower = message.to_lowercase();
    for phrase in phrases {
        if let Some(start) = lower.find(phrase).map(|i| i + phrase.len()) {
            let value = message
                .get(start..)?
                .split(['.', ';', '\n', '!', '?'])
                .next()?
                .trim()
                .trim_matches(['"', '\'', '`']);
            if !value.is_empty() {
                return Some(value.to_string());
            }
        }
    }
    None
}

fn clean_value(value: &str, max_chars: usize) -> Option<String> {
    let value = value
        .chars()
        .filter(|ch| !ch.is_control())
        .take(max_chars)
        .collect::<String>()
        .trim()
        .to_string();
    if value.is_empty() {
        None
    } else {
        Some(value)
    }
}

fn country_name_to_code(country: &str) -> Option<&'static str> {
    match country.trim().trim_end_matches('.').to_lowercase().as_str() {
        "bo" | "bolivia" => Some("BO"),
        "cl" | "chile" => Some("CL"),
        "ar" | "argentina" => Some("AR"),
        "pe" | "perú" | "peru" => Some("PE"),
        "br" | "brasil" | "brazil" => Some("BR"),
        "co" | "colombia" => Some("CO"),
        "ec" | "ecuador" => Some("EC"),
        "py" | "paraguay" => Some("PY"),
        "uy" | "uruguay" => Some("UY"),
        "ve" | "venezuela" => Some("VE"),
        "mx" | "méxico" | "mexico" => Some("MX"),
        "us" | "estados unidos" | "united states" => Some("US"),
        "ca" | "canadá" | "canada" => Some("CA"),
        "es" | "españa" | "espana" | "spain" => Some("ES"),
        "cr" | "costa rica" => Some("CR"),
        "pa" | "panamá" | "panama" => Some("PA"),
        "gt" | "guatemala" => Some("GT"),
        "hn" | "honduras" => Some("HN"),
        "sv" | "el salvador" => Some("SV"),
        "ni" | "nicaragua" => Some("NI"),
        "do" | "república dominicana" | "republica dominicana" => Some("DO"),
        "cu" | "cuba" => Some("CU"),
        "pr" | "puerto rico" => Some("PR"),
        "gb" | "reino unido" | "united kingdom" => Some("GB"),
        "fr" | "francia" | "france" => Some("FR"),
        "de" | "alemania" | "germany" => Some("DE"),
        "it" | "italia" | "italy" => Some("IT"),
        "pt" | "portugal" => Some("PT"),
        "jp" | "japón" | "japon" | "japan" => Some("JP"),
        "au" | "australia" => Some("AU"),
        _ => None,
    }
}

fn sanitize(mut profile: UserProfile) -> Result<UserProfile, String> {
    profile.display_name = profile.display_name.and_then(|v| clean_value(&v, 60));
    profile.city = profile.city.and_then(|v| clean_value(&v, 80));
    profile.home_country = profile.home_country.and_then(|v| {
        let v = v.trim().to_ascii_uppercase();
        if v.is_empty() {
            None
        } else {
            Some(v)
        }
    });
    profile.preferred_language = profile.preferred_language.and_then(|v| clean_value(&v, 20));
    profile.time_zone = profile.time_zone.and_then(|v| clean_value(&v, 80));
    profile.response_style = profile.response_style.and_then(|v| clean_value(&v, 40));
    if let Some(country) = profile.home_country.as_deref() {
        profile.home_country = Some(validate_country(country)?);
    }
    if let Some(language) = profile.preferred_language.as_deref() {
        if !["es", "en", "pt"].contains(&language) {
            return Err("El idioma debe ser es, en o pt.".into());
        }
    }
    if let Some(style) = profile.response_style.as_deref() {
        if !["conciso", "detallado", "paso a paso"].contains(&style) {
            return Err("El estilo de respuesta seleccionado no es válido.".into());
        }
    }
    Ok(profile)
}

fn validate_country(country: &str) -> Result<String, String> {
    let code = country.trim().to_ascii_uppercase();
    if code.len() != 2 || !code.bytes().all(|byte| byte.is_ascii_uppercase()) {
        return Err("Usa el código de país de dos letras, por ejemplo BO o CL.".into());
    }
    Ok(code)
}

fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<Option<T>, String> {
    match std::fs::read_to_string(path) {
        Ok(value) => serde_json::from_str(&value)
            .map(Some)
            .map_err(|e| format!("No se pudo leer la configuración local: {e}")),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("No se pudo abrir la configuración local: {e}")),
    }
}

fn write_json_atomic<T: Serialize>(path: &Path, value: &T) -> Result<(), String> {
    let bytes =
        serde_json::to_vec_pretty(value).map_err(|e| format!("No se pudo serializar: {e}"))?;
    let temp = path.with_extension(format!("{}.tmp", uuid::Uuid::new_v4()));
    std::fs::write(&temp, bytes).map_err(|e| format!("No se pudo escribir el temporal: {e}"))?;
    replace_file(&temp, path).map_err(|e| {
        let _ = std::fs::remove_file(&temp);
        format!("No se pudo guardar el perfil local: {e}")
    })
}

#[cfg(windows)]
fn replace_file(source: &Path, destination: &Path) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use std::ptr;

    // ReplaceFileW preserves an existing destination atomically; MoveFileExW
    // handles first save. Both paths stay in the same local data directory.
    #[link(name = "kernel32")]
    extern "system" {
        fn ReplaceFileW(
            replaced: *const u16,
            replacement: *const u16,
            backup: *const u16,
            flags: u32,
            exclude: *const core::ffi::c_void,
            reserved: *const core::ffi::c_void,
        ) -> i32;
        fn MoveFileExW(existing: *const u16, new: *const u16, flags: u32) -> i32;
        fn GetLastError() -> u32;
    }
    let wide = |path: &Path| {
        path.as_os_str()
            .encode_wide()
            .chain(Some(0))
            .collect::<Vec<_>>()
    };
    let source_wide = wide(source);
    let destination_wide = wide(destination);
    let replacement_ok = unsafe {
        ReplaceFileW(
            destination_wide.as_ptr(),
            source_wide.as_ptr(),
            ptr::null(),
            0x0000_0001,
            ptr::null(),
            ptr::null(),
        )
    };
    if replacement_ok != 0 {
        return Ok(());
    }

    // ERROR_FILE_NOT_FOUND/PATH_NOT_FOUND means this is the first save. For
    // other failures, return the OS error instead of risking deleting data.
    let error = unsafe { GetLastError() };
    if error != 2 && error != 3 {
        return Err(std::io::Error::from_raw_os_error(error as i32));
    }
    let move_ok = unsafe {
        MoveFileExW(
            source_wide.as_ptr(),
            destination_wide.as_ptr(),
            0x0000_0001 | 0x0000_0008,
        )
    };
    if move_ok != 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

#[cfg(not(windows))]
fn replace_file(source: &Path, destination: &Path) -> std::io::Result<()> {
    std::fs::rename(source, destination)
}

fn workspace_key(workspace: &str) -> String {
    let path = Path::new(workspace)
        .canonicalize()
        .unwrap_or_else(|_| PathBuf::from(workspace));
    let mut normalized = path.to_string_lossy().trim().to_string();
    #[cfg(windows)]
    {
        normalized = normalized.to_lowercase();
    }
    format!("{:x}", Sha256::digest(normalized.as_bytes()))
}

fn load_project_country(root: &Path, workspace: &str) -> Result<Option<String>, String> {
    let contexts =
        read_json::<HashMap<String, String>>(&root.join(PROJECT_FILE))?.unwrap_or_default();
    Ok(contexts.get(&workspace_key(workspace)).cloned())
}

#[cfg(windows)]
fn windows_home_region() -> Option<String> {
    use std::os::windows::ffi::OsStringExt;
    #[link(name = "kernel32")]
    extern "system" {
        fn GetUserDefaultGeoName(buffer: *mut u16, count: i32) -> i32;
    }
    let mut buffer = [0u16; 16];
    // Reads the configured Windows region only; no GPS, network lookup, or permission request.
    let length = unsafe { GetUserDefaultGeoName(buffer.as_mut_ptr(), buffer.len() as i32) };
    if length <= 1 || length as usize > buffer.len() {
        return None;
    }
    let code = std::ffi::OsString::from_wide(&buffer[..length as usize - 1])
        .to_string_lossy()
        .to_ascii_uppercase();
    validate_country(&code).ok()
}

#[cfg(not(windows))]
fn windows_home_region() -> Option<String> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    fn temp_root() -> PathBuf {
        let root = std::env::temp_dir().join(format!("aura-profile-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    #[test]
    fn profile_round_trips_only_the_allowlisted_personal_fields() {
        let root = temp_root();
        let profile = sanitize(UserProfile {
            display_name: Some("Yemsy".into()),
            home_country: Some("bo".into()),
            preferred_language: Some("es".into()),
            ..Default::default()
        })
        .unwrap();
        write_json_atomic(&root.join(PROFILE_FILE), &profile).unwrap();
        let updated = UserProfile {
            display_name: Some("Ana María".into()),
            ..profile.clone()
        };
        write_json_atomic(&root.join(PROFILE_FILE), &updated).unwrap();
        let raw = std::fs::read_to_string(root.join(PROFILE_FILE)).unwrap();
        assert!(!raw.contains("historial"));
        assert_eq!(
            read_json::<UserProfile>(&root.join(PROFILE_FILE)).unwrap(),
            Some(updated)
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn auto_memory_is_opt_in_and_only_accepts_clear_self_disclosures() {
        let mut profile = UserProfile::default();
        assert!(!remember_explicit_facts(&mut profile, "Me llamo Ana."));
        profile.auto_remember_facts = true;
        assert!(remember_explicit_facts(&mut profile, "Me llamo Ana."));
        assert_eq!(profile.display_name.as_deref(), Some("Ana"));
        assert!(remember_explicit_facts(&mut profile, "Mi país es Bolivia."));
        assert_eq!(profile.home_country.as_deref(), Some("BO"));
        assert!(remember_explicit_facts(
            &mut profile,
            "Vivo en Santa Cruz, Bolivia."
        ));
        assert_eq!(profile.city.as_deref(), Some("Santa Cruz"));
        assert!(!remember_explicit_facts(
            &mut profile,
            "Crea esta app para Chile."
        ));
        assert_eq!(profile.home_country.as_deref(), Some("BO"));
    }

    #[test]
    fn project_country_is_scoped_by_a_path_hash_and_can_be_removed() {
        let root = temp_root();
        save_project_country_at(&root, r"C:\work\app", Some("CL".into())).unwrap();
        let raw = std::fs::read_to_string(root.join(PROJECT_FILE)).unwrap();
        assert!(!raw.contains(r"C:\work\app"));
        assert_eq!(
            load_project_country(&root, r"C:\work\app")
                .unwrap()
                .as_deref(),
            Some("CL")
        );
        save_project_country_at(&root, r"C:\work\app", None).unwrap();
        assert_eq!(load_project_country(&root, r"C:\work\app").unwrap(), None);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn invalid_profile_fields_are_rejected() {
        assert!(validate_country("BOL").is_err());
        assert!(sanitize(UserProfile {
            preferred_language: Some("xx".into()),
            ..Default::default()
        })
        .is_err());
    }

    #[test]
    fn windows_region_is_only_a_suggestion_and_project_country_is_separate() {
        let context = prompt_context(&ProfileSnapshot {
            profile: UserProfile {
                home_country: Some("BO".into()),
                ..Default::default()
            },
            windows_region_suggestion: Some("CL".into()),
            project_country: Some("PE".into()),
        });
        assert!(context.contains("país de referencia del usuario (ISO alpha-2): BO"));
        assert!(context.contains("país objetivo guardado para este proyecto: PE"));
        assert!(!context.contains("REGIÓN SUGERIDA POR WINDOWS"));

        let suggestion = prompt_context(&ProfileSnapshot {
            profile: UserProfile::default(),
            windows_region_suggestion: Some("CL".into()),
            project_country: None,
        });
        assert!(suggestion.contains("NO CONFIRMADA"));
        assert!(suggestion.contains("no una ubicación física verificada"));
    }
}
