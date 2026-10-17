/** Confirmación nativa de Windows antes de algo que no se deshace solo. `window.confirm` no sirve en Tauri: devuelve
 * una Promise (que cuenta como «sí») y la acción seguía sin preguntar. Si el diálogo falla, la respuesta es «no». */
export async function confirmar(texto: string, peligro = true): Promise<boolean> {
  try {
    const { ask } = await import('@tauri-apps/plugin-dialog');
    return await ask(texto, { title: 'Aura Sentinel', kind: peligro ? 'warning' : 'info', okLabel: 'Sí', cancelLabel: 'Cancelar' });
  } catch (e) {
    console.error('No se pudo mostrar la confirmación:', e);
    return false;
  }
}
