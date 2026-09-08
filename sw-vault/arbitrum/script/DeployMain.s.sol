// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {Script, console2} from "forge-std/Script.sol";
import {SwVault} from "../src/SwVault.sol";

/// Arbitrum One vault. Circle USDC. Broadcast with EVM_KEY.
contract DeployMain is Script {
    address internal constant CIRCLE_USDC = 0xaf88d065e77c8cC2239327C5EDb3A432268e5831;

    function run() external {
        vm.startBroadcast();
        SwVault vault = new SwVault(CIRCLE_USDC, msg.sender);
        vm.stopBroadcast();
        console2.log("USDC", CIRCLE_USDC);
        console2.log("SwVault", address(vault));
        console2.log("platform", msg.sender);
    }
}
