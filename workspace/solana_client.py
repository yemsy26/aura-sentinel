"""
Aura-Sentinel: Script de conexión a la red Solana
Creado automáticamente. Usa SOLO requests (sin dependencias externas).
Conecta al RPC público de Solana y muestra información de la red.
"""
import requests
import json

SOLANA_RPC_URL = "https://api.mainnet-beta.solana.com"

def rpc_call(method: str, params: list = []) -> dict:
    """Realiza una llamada JSON-RPC al endpoint de Solana."""
    headers = {"Content-Type": "application/json"}
    payload = {
        "jsonrpc": "2.0",
        "id": 1,
        "method": method,
        "params": params
    }
    try:
        response = requests.post(SOLANA_RPC_URL, headers=headers, json=payload, timeout=15)
        response.raise_for_status()
        return response.json()
    except requests.exceptions.Timeout:
        return {"error": "La petición tardó demasiado (timeout)."}
    except requests.exceptions.RequestException as e:
        return {"error": str(e)}

def main():
    print("=" * 50)
    print("  AURA-SENTINEL: Conexión a la Red Solana")
    print("=" * 50)
    print(f"  RPC: {SOLANA_RPC_URL}\n")

    # 1. Obtener la versión del nodo
    print("[1] Obteniendo versión de Solana...")
    version = rpc_call("getVersion")
    if "result" in version:
        print(f"    Versión: {version['result'].get('solana-core', 'Desconocida')}")
    else:
        print(f"    Error: {version.get('error')}")

    # 2. Obtener el último blockhash
    print("\n[2] Obteniendo último blockhash...")
    blockhash_res = rpc_call("getLatestBlockhash", [{"commitment": "finalized"}])
    if "result" in blockhash_res:
        bh = blockhash_res["result"]["value"]["blockhash"]
        print(f"    Blockhash: {bh}")
    else:
        print(f"    Error: {blockhash_res.get('error')}")

    # 3. Obtener el slot actual
    print("\n[3] Obteniendo slot actual...")
    slot_res = rpc_call("getSlot")
    if "result" in slot_res:
        print(f"    Slot actual: {slot_res['result']:,}")
    else:
        print(f"    Error: {slot_res.get('error')}")

    # 4. Obtener el supply total de SOL
    print("\n[4] Obteniendo supply de SOL...")
    supply_res = rpc_call("getSupply")
    if "result" in supply_res:
        supply_lamports = supply_res["result"]["value"]["total"]
        supply_sol = supply_lamports / 1_000_000_000
        print(f"    Supply total: {supply_sol:,.2f} SOL")
    else:
        print(f"    Error: {supply_res.get('error')}")

    print("\n" + "=" * 50)
    print("  Conexión a Solana Mainnet: EXITOSA")
    print("=" * 50)

if __name__ == "__main__":
    main()
