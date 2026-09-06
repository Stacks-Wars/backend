/**
 * Deploy the Solana play mint + vault program on devnet.
 *
 *   cd frontend && node ../backend/sw-vault/solana/scripts/deploy-play.mjs
 */
import { execFileSync, spawnSync } from "node:child_process"
import { existsSync, readFileSync, writeFileSync } from "node:fs"
import { dirname, resolve } from "node:path"
import { fileURLToPath } from "node:url"

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..")
const PLAY = resolve(ROOT, "play")
const WALLET = resolve(PLAY, "dev-wallet.json")
const PROGRAM_KEY = resolve(PLAY, "program.json")
const SO = resolve(PLAY, "sw-vault.so")
const WARS_PROGRAM = "8NZHj9VH9JkqiAg19CK43ZLuK5hn5jXPBnLfbeKonqfy"
const RPC = "https://api.devnet.solana.com"

function sh(bin, args) {
    return execFileSync(bin, args, { encoding: "utf8" }).trim()
}

if (!existsSync(WALLET)) {
    throw new Error(`missing ${WALLET} — generate the play wallet first`)
}

const pubkey = sh("solana", [
    "address",
    "--keypair",
    WALLET,
    "--url",
    RPC,
])
console.log(`play wallet ${pubkey}`)

function solBalance() {
    const out = sh("solana", ["balance", pubkey, "--url", RPC])
    return Number.parseFloat(out)
}

let balance = solBalance()
for (let i = 0; i < 6 && balance < 3; i++) {
    try {
        console.log(sh("solana", ["airdrop", "2", pubkey, "--url", RPC]))
    } catch (err) {
        console.log(`airdrop ${i}: ${err.message?.slice(0, 160)}`)
    }
    await new Promise((r) => setTimeout(r, 4000))
    balance = solBalance()
    console.log(`sol ${balance}`)
}
if (balance < 2) {
    throw new Error(`play wallet ${pubkey} needs ~2 SOL on devnet`)
}

const MINT_KEY = resolve(PLAY, "mint.json")
if (!existsSync(MINT_KEY)) {
    sh("solana-keygen", [
        "new",
        "--no-bip39-passphrase",
        "--silent",
        "--outfile",
        MINT_KEY,
    ])
}
const mint = sh("solana-keygen", ["pubkey", MINT_KEY])
const mintOut = sh("spl-token", [
    "create-token",
    "--decimals",
    "6",
    "--fee-payer",
    WALLET,
    "--mint-authority",
    WALLET,
    "--url",
    RPC,
    MINT_KEY,
])
console.log(mintOut)

if (!existsSync(PROGRAM_KEY)) {
    sh("solana-keygen", [
        "new",
        "--no-bip39-passphrase",
        "--silent",
        "--outfile",
        PROGRAM_KEY,
    ])
}
const programId = sh("solana-keygen", ["pubkey", PROGRAM_KEY])
console.log(`program ${programId}`)

console.log(sh("solana", ["program", "dump", "-u", "devnet", WARS_PROGRAM, SO]))
console.log(
    sh("solana", [
        "program",
        "deploy",
        "--url",
        RPC,
        "--keypair",
        WALLET,
        "--program-id",
        PROGRAM_KEY,
        SO,
    ])
)

const frontend = resolve(ROOT, "../../../frontend")
const init = spawnSync(
    process.execPath,
    [resolve(frontend, "scripts/initialize-solana-vault.mjs")],
    {
        cwd: frontend,
        stdio: "inherit",
        env: {
            ...process.env,
            SOLANA_KEY: readFileSync(resolve(PLAY, "mnemonic.txt"), "utf8").trim(),
            SOLANA_USDC_MINT: mint,
            SOLANA_VAULT_PROGRAM_ID: programId,
            SOLANA_NETWORK: "devnet",
            SOLANA_RPC_URL: RPC,
        },
    }
)
if (init.status) process.exit(init.status ?? 1)

writeFileSync(
    resolve(PLAY, "ids.env"),
    [
        `DEV_SOLANA_MNEMONIC=${readFileSync(resolve(PLAY, "mnemonic.txt"), "utf8").trim()}`,
        `DEV_SOLANA_PLATFORM_WALLET=${pubkey}`,
        `DEV_SOLANA_USDC_MINT=${mint}`,
        `DEV_SOLANA_VAULT_PROGRAM_ID=${programId}`,
        "",
    ].join("\n")
)
console.log(`wrote ${resolve(PLAY, "ids.env")}`)
console.log(`DEV_SOLANA_USDC_MINT=${mint}`)
console.log(`DEV_SOLANA_VAULT_PROGRAM_ID=${programId}`)
console.log(`DEV_SOLANA_PLATFORM_WALLET=${pubkey}`)
