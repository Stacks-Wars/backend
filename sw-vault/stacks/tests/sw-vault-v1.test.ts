import { Cl } from "@stacks/transactions";
import { createHash } from "crypto";
import { describe, expect, it, beforeEach } from "vitest";
import {
  tupleCV,
  stringAsciiCV,
  uintCV,
  principalCV,
  serializeCV,
  signMessageHashRsv,
  privateKeyToPublic,
} from "@stacks/transactions";

const accounts = simnet.getAccounts();
const deployer = accounts.get("deployer")!;
const wallet1 = accounts.get("wallet_1")!;
const wallet2 = accounts.get("wallet_2")!;
const wallet3 = accounts.get("wallet_3")!;

/** Clarinet wallet_1 secret (matches TRUSTED-PUBLIC-KEY in contract). */
const ORACLE_SECRET =
  "7287ba251d44a4d3fd9276c88ce34c5c52a038955511cccaf77e61068649c17801";

const PATH = "abc123lobby";
const ENTRY = 10_000_000;
const USDCX_ASSET = "SP120SBRBQJ00MCWS7TM5R8WJNTTKD5K0HFRC2CNE.usdcx.usdcx-token";
const PLATFORM = "SP299MBHT7FPPP2SKEY73V4DHW67467SED87A4HH4";

function signOracle(params: {
  action: string;
  path: string;
  player: string;
  amount: number;
  nonce: number;
}): string {
  const message = tupleCV({
    action: stringAsciiCV(params.action),
    "lobby-path": stringAsciiCV(params.path),
    player: principalCV(params.player),
    amount: uintCV(params.amount),
    nonce: uintCV(params.nonce),
  });
  const serialized = serializeCV(message);
  const buffer = Buffer.from(
    typeof serialized === "string" ? serialized : Buffer.from(serialized).toString("hex"),
    "hex"
  );
  const hash = createHash("sha256").update(buffer).digest();
  const sig = signMessageHashRsv({
    messageHash: hash.toString("hex"),
    privateKey: ORACLE_SECRET,
  });
  return typeof sig === "string" ? sig : String(sig);
}

function signClaimOracle(params: {
  path: string;
  player: string;
  amount: number;
  nonce: number;
  devWallet: string;
  devFee: number;
}): string {
  const message = tupleCV({
    action: stringAsciiCV("claim"),
    "lobby-path": stringAsciiCV(params.path),
    player: principalCV(params.player),
    amount: uintCV(params.amount),
    nonce: uintCV(params.nonce),
    "dev-wallet": principalCV(params.devWallet),
    "dev-fee": uintCV(params.devFee),
  });
  const serialized = serializeCV(message);
  const buffer = Buffer.from(
    typeof serialized === "string" ? serialized : Buffer.from(serialized).toString("hex"),
    "hex"
  );
  const hash = createHash("sha256").update(buffer).digest();
  const sig = signMessageHashRsv({
    messageHash: hash.toString("hex"),
    privateKey: ORACLE_SECRET,
  });
  return typeof sig === "string" ? sig : String(sig);
}

function mintTo(recipient: string, amount: number) {
  simnet.mintFT(USDCX_ASSET, recipient, BigInt(amount));
}

describe("sw-vault-v1", () => {
  beforeEach(() => {
    mintTo(wallet1, 100_000_000);
    mintTo(wallet2, 100_000_000);
    mintTo(wallet3, 100_000_000);
  });

  it("oracle pubkey matches wallet_1", () => {
    expect(privateKeyToPublic(ORACLE_SECRET)).toBe(
      "03cd2cfdbd2ad9332828a7a13ef62cb999e063421c708e863a7ffed71fb61c88c9"
    );
  });

  it("first join sets entry; subsequent normal joins pay", () => {
    let r = simnet.callPublicFn(
      "sw-vault-v1",
      "join",
      [Cl.stringAscii(PATH), Cl.uint(ENTRY), Cl.bool(false)],
      wallet1
    );
    expect(r.result).toBeOk(Cl.bool(true));

    r = simnet.callPublicFn(
      "sw-vault-v1",
      "join",
      [Cl.stringAscii(PATH), Cl.uint(ENTRY), Cl.bool(false)],
      wallet2
    );
    expect(r.result).toBeOk(Cl.bool(true));

    const pot = simnet.callReadOnlyFn(
      "sw-vault-v1",
      "get-pot",
      [Cl.stringAscii(PATH)],
      deployer
    );
    expect(pot.result).toBeUint(ENTRY * 2);
  });

  it("sponsored follow-ups do not transfer", () => {
    let r = simnet.callPublicFn(
      "sw-vault-v1",
      "join",
      [Cl.stringAscii(PATH), Cl.uint(ENTRY), Cl.bool(true)],
      wallet1
    );
    expect(r.result).toBeOk(Cl.bool(true));

    r = simnet.callPublicFn(
      "sw-vault-v1",
      "join",
      [Cl.stringAscii(PATH), Cl.uint(ENTRY), Cl.bool(true)],
      wallet2
    );
    expect(r.result).toBeOk(Cl.bool(true));

    const pot = simnet.callReadOnlyFn(
      "sw-vault-v1",
      "get-pot",
      [Cl.stringAscii(PATH)],
      deployer
    );
    expect(pot.result).toBeUint(ENTRY);

    const paid = simnet.callReadOnlyFn(
      "sw-vault-v1",
      "get-paid",
      [Cl.stringAscii(PATH), Cl.principal(wallet2)],
      deployer
    );
    expect(paid.result).toBeSome(Cl.uint(0));
  });

  it("rejects wrong entry and sponsored mismatch", () => {
    simnet.callPublicFn(
      "sw-vault-v1",
      "join",
      [Cl.stringAscii(PATH), Cl.uint(ENTRY), Cl.bool(false)],
      wallet1
    );
    let r = simnet.callPublicFn(
      "sw-vault-v1",
      "join",
      [Cl.stringAscii(PATH), Cl.uint(ENTRY + 1), Cl.bool(false)],
      wallet2
    );
    expect(r.result).toBeErr(Cl.uint(204));

    r = simnet.callPublicFn(
      "sw-vault-v1",
      "join",
      [Cl.stringAscii(PATH), Cl.uint(ENTRY), Cl.bool(true)],
      wallet2
    );
    expect(r.result).toBeErr(Cl.uint(205));
  });

  it("leave refunds with valid sig; creator must be last", () => {
    simnet.callPublicFn(
      "sw-vault-v1",
      "join",
      [Cl.stringAscii(PATH), Cl.uint(ENTRY), Cl.bool(false)],
      wallet1
    );
    simnet.callPublicFn(
      "sw-vault-v1",
      "join",
      [Cl.stringAscii(PATH), Cl.uint(ENTRY), Cl.bool(false)],
      wallet2
    );

    const creatorLeaveSig = signOracle({
      action: "leave",
      path: PATH,
      player: wallet1,
      amount: ENTRY,
      nonce: 1,
    });
    let r = simnet.callPublicFn(
      "sw-vault-v1",
      "leave",
      [
        Cl.stringAscii(PATH),
        Cl.uint(1),
        Cl.bufferFromHex(creatorLeaveSig),
      ],
      wallet1
    );
    expect(r.result).toBeErr(Cl.uint(208));

    const leaveSig = signOracle({
      action: "leave",
      path: PATH,
      player: wallet2,
      amount: ENTRY,
      nonce: 2,
    });
    r = simnet.callPublicFn(
      "sw-vault-v1",
      "leave",
      [Cl.stringAscii(PATH), Cl.uint(2), Cl.bufferFromHex(leaveSig)],
      wallet2
    );
    expect(r.result).toBeOk(Cl.bool(true));
  });

  it("kick refunds participant", () => {
    simnet.callPublicFn(
      "sw-vault-v1",
      "join",
      [Cl.stringAscii(PATH), Cl.uint(ENTRY), Cl.bool(true)],
      wallet1
    );
    simnet.callPublicFn(
      "sw-vault-v1",
      "join",
      [Cl.stringAscii(PATH), Cl.uint(ENTRY), Cl.bool(true)],
      wallet2
    );

    const kickSig = signOracle({
      action: "kick",
      path: PATH,
      player: wallet2,
      amount: 0,
      nonce: 1,
    });
    const r = simnet.callPublicFn(
      "sw-vault-v1",
      "kick",
      [
        Cl.stringAscii(PATH),
        Cl.principal(wallet2),
        Cl.uint(1),
        Cl.bufferFromHex(kickSig),
      ],
      wallet1
    );
    expect(r.result).toBeOk(Cl.bool(true));
  });

  it("claim splits pot to winner + platform and freezes leave", () => {
    simnet.callPublicFn(
      "sw-vault-v1",
      "join",
      [Cl.stringAscii(PATH), Cl.uint(ENTRY), Cl.bool(false)],
      wallet1
    );
    simnet.callPublicFn(
      "sw-vault-v1",
      "join",
      [Cl.stringAscii(PATH), Cl.uint(ENTRY), Cl.bool(false)],
      wallet2
    );

    const pot = ENTRY * 2;
    const platform = Math.floor((pot * 2) / 100);
    const winnerAmt = pot - platform;

    const claimSig = signClaimOracle({
      path: PATH,
      player: wallet1,
      amount: pot,
      nonce: 10,
      devWallet: wallet3,
      devFee: 0,
    });
    let r = simnet.callPublicFn(
      "sw-vault-v1",
      "claim",
      [
        Cl.stringAscii(PATH),
        Cl.uint(pot),
        Cl.uint(10),
        Cl.principal(wallet3),
        Cl.uint(0),
        Cl.bufferFromHex(claimSig),
      ],
      wallet1
    );
    expect(r.result).toBeOk(Cl.bool(true));
    const ftEvents = r.events.filter((e) => e.event === "ft_transfer_event");
    expect(ftEvents).toHaveLength(2);
    expect(ftEvents[0]?.data.amount).toBe(String(winnerAmt));
    expect(ftEvents[0]?.data.recipient).toBe(wallet1);
    expect(ftEvents[1]?.data.amount).toBe(String(platform));
    expect(ftEvents[1]?.data.recipient).toBe(PLATFORM);

    const potLeft = simnet.callReadOnlyFn(
      "sw-vault-v1",
      "get-pot",
      [Cl.stringAscii(PATH)],
      deployer
    );
    expect(potLeft.result).toBeUint(0);

    const leaveSig = signOracle({
      action: "leave",
      path: PATH,
      player: wallet2,
      amount: ENTRY,
      nonce: 12,
    });
    r = simnet.callPublicFn(
      "sw-vault-v1",
      "leave",
      [Cl.stringAscii(PATH), Cl.uint(12), Cl.bufferFromHex(leaveSig)],
      wallet2
    );
    expect(r.result).toBeErr(Cl.uint(209));
  });

  it("claim with dev fee pays winner platform and dev", () => {
    simnet.callPublicFn(
      "sw-vault-v1",
      "join",
      [Cl.stringAscii(PATH), Cl.uint(ENTRY), Cl.bool(false)],
      wallet1
    );
    simnet.callPublicFn(
      "sw-vault-v1",
      "join",
      [Cl.stringAscii(PATH), Cl.uint(ENTRY), Cl.bool(false)],
      wallet2
    );

    const pot = ENTRY * 2;
    const platform = Math.floor((pot * 2) / 100);
    const dev = Math.floor((pot * 5) / 100);
    const winnerAmt = pot - platform - dev;

    const claimSig = signClaimOracle({
      path: PATH,
      player: wallet1,
      amount: pot,
      nonce: 1,
      devWallet: wallet3,
      devFee: 5,
    });
    const r = simnet.callPublicFn(
      "sw-vault-v1",
      "claim",
      [
        Cl.stringAscii(PATH),
        Cl.uint(pot),
        Cl.uint(1),
        Cl.principal(wallet3),
        Cl.uint(5),
        Cl.bufferFromHex(claimSig),
      ],
      wallet1
    );
    expect(r.result).toBeOk(Cl.bool(true));
    const ftEvents = r.events.filter((e) => e.event === "ft_transfer_event");
    expect(ftEvents).toHaveLength(3);
    expect(ftEvents[0]?.data.amount).toBe(String(winnerAmt));
    expect(ftEvents[0]?.data.recipient).toBe(wallet1);
    expect(ftEvents[1]?.data.amount).toBe(String(platform));
    expect(ftEvents[1]?.data.recipient).toBe(PLATFORM);
    expect(ftEvents[2]?.data.amount).toBe(String(dev));
    expect(ftEvents[2]?.data.recipient).toBe(wallet3);
  });

  it("rejects claim over pot and nonce replay", () => {
    simnet.callPublicFn(
      "sw-vault-v1",
      "join",
      [Cl.stringAscii(PATH), Cl.uint(ENTRY), Cl.bool(false)],
      wallet1
    );

    const over = signClaimOracle({
      path: PATH,
      player: wallet1,
      amount: ENTRY + 1,
      nonce: 1,
      devWallet: wallet3,
      devFee: 0,
    });
    let r = simnet.callPublicFn(
      "sw-vault-v1",
      "claim",
      [
        Cl.stringAscii(PATH),
        Cl.uint(ENTRY + 1),
        Cl.uint(1),
        Cl.principal(wallet3),
        Cl.uint(0),
        Cl.bufferFromHex(over),
      ],
      wallet1
    );
    expect(r.result).toBeErr(Cl.uint(211));

    const ok = signClaimOracle({
      path: PATH,
      player: wallet1,
      amount: ENTRY,
      nonce: 2,
      devWallet: wallet3,
      devFee: 0,
    });
    r = simnet.callPublicFn(
      "sw-vault-v1",
      "claim",
      [
        Cl.stringAscii(PATH),
        Cl.uint(ENTRY),
        Cl.uint(2),
        Cl.principal(wallet3),
        Cl.uint(0),
        Cl.bufferFromHex(ok),
      ],
      wallet1
    );
    expect(r.result).toBeOk(Cl.bool(true));

    r = simnet.callPublicFn(
      "sw-vault-v1",
      "claim",
      [
        Cl.stringAscii(PATH),
        Cl.uint(ENTRY),
        Cl.uint(2),
        Cl.principal(wallet3),
        Cl.uint(0),
        Cl.bufferFromHex(ok),
      ],
      wallet1
    );
    expect(r.result).toBeErr(Cl.uint(212));
  });

  it("calculate-split returns platform 2% and optional dev", () => {
    const amount = 100_000_000;
    let split = simnet.callReadOnlyFn(
      "sw-vault-v1",
      "calculate-split",
      [Cl.uint(amount), Cl.uint(0)],
      deployer
    );
    expect(split.result).toBeOk(
      Cl.tuple({
        platform: Cl.uint(2_000_000),
        dev: Cl.uint(0),
        winner: Cl.uint(98_000_000),
      })
    );

    split = simnet.callReadOnlyFn(
      "sw-vault-v1",
      "calculate-split",
      [Cl.uint(amount), Cl.uint(5)],
      deployer
    );
    expect(split.result).toBeOk(
      Cl.tuple({
        platform: Cl.uint(2_000_000),
        dev: Cl.uint(5_000_000),
        winner: Cl.uint(93_000_000),
      })
    );
  });
});
