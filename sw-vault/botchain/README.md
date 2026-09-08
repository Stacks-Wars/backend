# BOT Chain vault

Lobby escrow on BOT Chain. Platform is `msg.sender` for every vault call and pays **BOT**. Players never need BOT on dest (ERC-2612 permit). Mainnet USDT has no permit, so join uses Uniswap Permit2 after a one-time `USDT.approve(Permit2)`.

Play token is **USDT**, not USDC.

| Cluster | Network | Token | Mint $50 |
| --- | --- | --- | --- |
| Dest | Bohr (`968`) | Play `Usdt.sol` | Yes |
| Main | BOT Chain (`677`) | Official USDT `0xaBabc7Ddc03e501d190C676BF3d92ef0e6e87a3C` | No |

Permit2 (both clusters): `0x000000000022D473030F116dDEE9F6B43aC78BA3`

`pathHash` is `keccak256(bytes(path))`. Claim requires a seated player and enough pot, then takes 2% platform and 0–5% dest. After the first claim, join / `leaveSeat` / kick freeze.

## Keys

Dest uses the committed play mnemonic in [`play/mnemonic.txt`](./play/mnemonic.txt) (shared with Arbitrum dest). Main uses `EVM_KEY` (wars). Do not commit the wars mnemonic.

## Deploy

```sh
cd backend/sw-vault/botchain
forge test
# dest — Bohr play USDT + vault
forge script script/DeployPlay.s.sol:DeployPlay \
  --rpc-url bohr --broadcast \
  --mnemonics play/mnemonic.txt \
  --mnemonic-derivation-paths "m/44'/60'/0'/0/0"
# main — vault against official USDT
forge script script/DeployMain.s.sol:DeployMain \
  --rpc-url mainnet --broadcast \
  --mnemonics "$EVM_KEY_FILE" \
  --mnemonic-derivation-paths "m/44'/60'/0'/0/0"
```

Hardcode the printed addresses into `DEV_BOTCHAIN_*` and `MAIN_BOTCHAIN_*` in `frontend/lib/config.ts` and `backend/crates/sw-server/src/config.rs`.

## Live (2026-09-08)

| | Token | Vault | Platform |
| --- | --- | --- | --- |
| Dest Bohr | [`0x5B161eeb…`](https://scan.bohr.life/address/0x5B161eebFE0352F510C2FDf6aFb58C034A99DBf1) play USDT | [`0xBEa87817…`](https://scan.bohr.life/address/0xBEa87817C05aB529E62BeAD046A151CdF46d1E19) | `0x2092cD00…` |
| Main | [`0xaBabc7Dd…`](https://scan.botchain.ai/address/0xaBabc7Ddc03e501d190C676BF3d92ef0e6e87a3C) USDT | [`0x9EbDA94b…`](https://scan.botchain.ai/address/0x9EbDA94b2DE11C001ea1120fA723e8C579376c9C) | `0xD456920A…` |
