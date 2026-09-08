// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {Script, console2} from "forge-std/Script.sol";
import {SwVault} from "../src/SwVault.sol";

address constant PERMIT2 = 0x000000000022D473030F116dDEE9F6B43aC78BA3;
address constant BOT_CHAIN_USDT = 0xaBabc7Ddc03e501d190C676BF3d92ef0e6e87a3C;

/// BOT Chain mainnet vault. Official USDT. Broadcast with EVM_KEY.
contract DeployMain is Script {
    function run() external {
        vm.startBroadcast();
        SwVault vault = new SwVault(BOT_CHAIN_USDT, msg.sender, PERMIT2);
        vm.stopBroadcast();
        console2.log("USDT", BOT_CHAIN_USDT);
        console2.log("SwVault", address(vault));
        console2.log("platform", msg.sender);
        console2.log("Permit2", PERMIT2);
    }
}
