# sw-vault-v1

Central USDCx escrow for Stacks Wars lobbies (normal + sponsored).

## Clarity 4 (mainnet)

Mainnet epoch uses **Clarity 4**. The old `as-contract` builtin was **removed**.

| Old (Clarity ≤3) | New (Clarity 4) |
| --- | --- |
| `(as-contract tx-sender)` | `current-contract` |
| `(as-contract (…))` | `(as-contract? ((with-ft …)) …)` |

In this contract that meant:

1. **transfer-in** recipient: `current-contract` (was `(as-contract tx-sender)`)
2. **transfer-out**: `(as-contract? ((with-ft 'SP120…usdcx "usdcx-token" amount)) …)`

Deploy with **Clarity version 4** (default on current mainnet tooling).

Token calls use the literal principal  
`'SP120SBRBQJ00MCWS7TM5R8WJNTTKD5K0HFRC2CNE.usdcx`  
(Clarity cannot `contract-call?` through a `define-constant`).

## Deploy

1. Derive compressed pubkey from `STACKS_WARS_KEY` (account 0) and bake it as `TRUSTED-PUBLIC-KEY`.
2. Deploy from `SP299MBHT7FPPP2SKEY73V4DHW67467SED87A4HH4` as **Clarity 4**
   (same principal as on-chain `PLATFORM-WALLET` for the 2% claim fee).
3. Set env on backend + frontend:

```
SW_VAULT_CONTRACT=SP299MBHT7FPPP2SKEY73V4DHW67467SED87A4HH4.sw-vault-v1
STACKS_WARS_KEY=<24-word mnemonic>
USDCX_ASSET_NAME=usdcx-token
HIRO_API_KEY=<hiro api key>
```

Paid lobbies always require a real vault join tx (no auto-confirm). Balance SoT is the official explorer for that chain (Hiro for this Stacks vault).

## Local tools

Clarinet **≥ 3.22** and `@stacks/clarinet-sdk` (not the old `@hirosystems/*` 3.8 packages) are required for Clarity 4 / `as-contract?`.

```bash
# optional local CLI
# download clarinet 3.22+ into .bin/clarinet

npm install
npm test
./.bin/clarinet check   # or: clarinet check
```
