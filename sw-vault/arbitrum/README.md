# Arbitrum vault

Lobby escrow. Platform is `msg.sender` for every vault call and pays gas. Players sign ERC-2612 permit; there is no `approve` + `join` path.

| Cluster | Network | Token | Mint $50 |
| --- | --- | --- | --- |
| Dest | Arbitrum Sepolia (`421614`) | Play `UsdCx.sol` | Yes |
| Main | Arbitrum One (`42161`) | Circle USDC | No |

`pathHash` is `keccak256(bytes(path))`. Claim requires a seated player and enough pot, then takes 2% platform and 0–5% dest. After the first claim, join / `leaveSeat` / kick freeze.

## Keys

Dest uses the committed play mnemonic in [`play/mnemonic.txt`](./play/mnemonic.txt). Main uses `ARBITRUM_KEY` (wars). Do not commit the wars mnemonic.

## Deploy

```sh
cd backend/sw-vault/arbitrum
forge test
# dest — Sepolia play token + vault
forge script script/DeployPlay.s.sol:DeployPlay \
  --rpc-url sepolia --broadcast \
  --mnemonics play/mnemonic.txt \
  --mnemonic-derivation-paths "m/44'/60'/0'/0/0"
# main — One vault against Circle USDC
forge script script/DeployMain.s.sol:DeployMain \
  --rpc-url one --broadcast \
  --mnemonics "$ARBITRUM_KEY_FILE" \
  --mnemonic-derivation-paths "m/44'/60'/0'/0/0"
```

Hardcode the printed addresses into `DEV_ARBITRUM_*` and `MAIN_ARBITRUM_*` in `frontend/lib/config.ts` and `backend/crates/sw-server/src/config.rs`.

## Live (2026-09-07)

| | Token | Vault | Platform |
| --- | --- | --- | --- |
| Dest Sepolia | [`0x554fF14e…`](https://sepolia.arbiscan.io/address/0x554fF14eaA5380a99e765a7748D6eb5A6B1AF8c7) | [`0xc704D03f…`](https://sepolia.arbiscan.io/address/0xc704D03f4B09bc21d7cdb36C5FFF9ccB862d612F) | `0x2092cD00…` |
| Main One | [`0xaf88d065…`](https://arbiscan.io/address/0xaf88d065e77c8cC2239327C5EDb3A432268e5831) Circle USDC | [`0x0B4379B2…`](https://arbiscan.io/address/0x0B4379B27050048D868376aDBA6a03826bDd6e90) | `0xD456920A…` |
