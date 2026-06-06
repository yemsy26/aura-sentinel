use std::path::Path;

/// Verifica que la ruta objetivo esté estrictamente dentro del workspace_path.
/// Previene ataques de Directory Traversal como '../../windows/system32'.
pub fn is_path_allowed(workspace_path: &Path, target_path: &Path) -> bool {
    // Resolver la ruta absoluta canonizada del workspace
    let workspace_canon = match std::fs::canonicalize(workspace_path) {
        Ok(p) => p,
        Err(_) => return false, // Si no se puede canonizar el workspace, abortamos por seguridad
    };

    // Para el archivo objetivo, si existe, lo canonizamos para verificar su ruta real.
    // Si no existe, simulamos la resolución absoluta asumiendo que estará dentro del workspace
    // (el intento de crear en ../ resolvería el string de ruta).
    let target_absolute = if target_path.is_absolute() {
        target_path.to_path_buf()
    } else {
        workspace_path.join(target_path)
    };

    // Si el archivo ya existe y podemos canonizarlo, verificamos su ubicación física real (resolviendo symlinks)
    if let Ok(target_canon) = std::fs::canonicalize(&target_absolute) {
        return target_canon.starts_with(&workspace_canon);
    }

    // Si no existe (es un archivo nuevo), verificamos la ruta sintáctica limpiada
    // Utilizaremos un método básico de limpieza de ".." si es posible, o simplemente 
    // forzamos a que, incluso sintácticamente, empiece con el workspace.
    // Para simplificar y mantener máxima seguridad:
    // Evitamos permitir cualquier ruta que contenga ".."
    let target_str = target_absolute.to_string_lossy();
    if target_str.contains("..\\") || target_str.contains("../") {
        return false;
    }

    target_absolute.starts_with(&workspace_path)
}
