// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {Script, console2} from "forge-std/Script.sol";
import {UsdCx} from "../src/UsdCx.sol";
import {SwVault} from "../src/SwVault.sol";

/// Dest Sepolia deploy. Broadcast with the committed play mnemonic.
contract DeployPlay is Script {
    function run() external {
        vm.startBroadcast();
        UsdCx usdc = new UsdCx(msg.sender);
        SwVault vault = new SwVault(address(usdc), msg.sender);
        vm.stopBroadcast();
        console2.log("USDCx", address(usdc));
        console2.log("SwVault", address(vault));
        console2.log("platform", msg.sender);
    }
}
