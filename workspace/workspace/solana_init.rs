#![allow(unused_imports)]

use solana_sdk::signature::{Keypair, Signer};
use solana_sdk::system_instruction;
use solana_sdk::transaction::Transaction;

fn main() {
    // Generar una nueva llave para el cuenta principal
    let keypair = Keypair::new();
    println!\"Tu clave privada es: {}\", keypair.to_base58_string());

    // Crear un nuevo cliente RPC de Solana (usando una URL pública)
    let rpc_client = solana_rpc_client::rpc_client::RpcClient::new(