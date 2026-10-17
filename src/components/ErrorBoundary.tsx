import { Component, type ErrorInfo, type ReactNode } from 'react';

/** Si un componente falla, React desmonta toda la interfaz: esto muestra el error y deja recargar en vez de una
 * ventana vacía. El agente y sus procesos siguen en el backend; recargar solo vuelve a pintar. */
export class ErrorBoundary extends Component<{ children: ReactNode }, { error: Error | null }> {
  state = { error: null as Error | null };

  static getDerivedStateFromError(error: Error) {
    return { error };
  }

  componentDidCatch(error: Error, info: ErrorInfo) {
    console.error('Fallo en la interfaz:', error, info.componentStack);
  }

  render() {
    if (!this.state.error) return this.props.children;
    return (
      <div className="welcome" role="alert">
        <h1>Algo falló en la interfaz</h1>
        <span>El agente y tus archivos no se ven afectados. Recarga para seguir.</span>
        <code className="dim" style={{ maxWidth: 640, whiteSpace: 'pre-wrap' }}>{String(this.state.error.message || this.state.error)}</code>
        <button className="btn primary" onClick={() => window.location.reload()}>Recargar la interfaz</button>
      </div>
    );
  }
}
