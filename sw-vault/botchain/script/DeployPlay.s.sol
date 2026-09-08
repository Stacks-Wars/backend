// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {Script, console2} from "forge-std/Script.sol";
import {Usdt} from "../src/Usdt.sol";
import {SwVault} from "../src/SwVault.sol";

address constant PERMIT2 = 0x000000000022D473030F116dDEE9F6B43aC78BA3;

/// Bohr dest deploy. Broadcast with the committed play mnemonic.
contract DeployPlay is Script {
    function run() external {
        vm.startBroadcast();
        Usdt usdt = new Usdt(msg.sender);
        SwVault vault = new SwVault(address(usdt), msg.sender, PERMIT2);
        vm.stopBroadcast();
        console2.log("USDT", address(usdt));
        console2.log("SwVault", address(vault));
        console2.log("platform", msg.sender);
        console2.log("Permit2", PERMIT2);
    }
}
