// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {Test} from "forge-std/Test.sol";
import {UsdCx} from "../src/UsdCx.sol";
import {SwVault} from "../src/SwVault.sol";

contract SwVaultTest is Test {
    UsdCx internal usdc;
    SwVault internal vault;
    address internal platform = address(this);
    uint256 internal playerPk = 1;
    address internal player = vm.addr(playerPk);
    bytes32 internal path = keccak256("01abcdef");

    function setUp() public {
        usdc = new UsdCx(platform);
        vault = new SwVault(address(usdc), platform);
        usdc.mint(player, 50_000_000);
    }

    function testJoinLeave() public {
        _joinWithPermit(player, playerPk, 5_000_000);
        (uint256 entry, uint256 pot, uint256 seats,,) = _lobby();
        assertEq(entry, 5_000_000);
        assertEq(pot, 5_000_000);
        assertEq(seats, 1);
        assertEq(usdc.balanceOf(player), 45_000_000);

        vault.leaveSeat(player, path);
        (, pot, seats,,) = _lobby();
        assertEq(pot, 0);
        assertEq(seats, 0);
        assertEq(usdc.balanceOf(player), 50_000_000);
    }

    function testClaimSplits() public {
        _joinWithPermit(player, playerPk, 10_000_000);
        address dest = vm.addr(2);
        vault.claim(player, path, 10_000_000, dest, 5);
        // 2% platform, 5% dest, rest winner
        assertEq(usdc.balanceOf(platform), 200_000);
        assertEq(usdc.balanceOf(dest), 500_000);
        assertEq(usdc.balanceOf(player), 49_300_000);
        (,,, bool claimsStarted,) = _lobby();
        assertTrue(claimsStarted);
        vm.expectRevert(SwVault.ClaimsStarted.selector);
        vault.leaveSeat(player, path);
    }

    function testClaimRequiresSeat() public {
        vm.expectRevert(SwVault.NotSeated.selector);
        vault.claim(player, path, 1_000_000, address(0), 0);
    }

    function testClaimRequiresPot() public {
        _joinWithPermit(player, playerPk, 5_000_000);
        vm.expectRevert(SwVault.InsufficientPot.selector);
        vault.claim(player, path, 5_000_001, address(0), 0);
    }

    function testKick() public {
        _joinWithPermit(player, playerPk, 5_000_000);
        vault.kick(player, path);
        assertEq(usdc.balanceOf(player), 50_000_000);
        assertEq(vault.seats(path, player), 0);
    }

    function testOnlyPlatform() public {
        vm.prank(player);
        vm.expectRevert(SwVault.NotPlatform.selector);
        vault.joinWithPermit(player, path, 1, block.timestamp + 1, 0, bytes32(0), bytes32(0));
    }

    function _joinWithPermit(address who, uint256 pk, uint256 amount) internal {
        uint256 nonce = usdc.nonces(who);
        uint256 deadline = block.timestamp + 1 hours;
        bytes32 digest = keccak256(
            abi.encodePacked(
                "\x19\x01",
                usdc.DOMAIN_SEPARATOR(),
                keccak256(abi.encode(usdc.PERMIT_TYPEHASH(), who, address(vault), amount, nonce, deadline))
            )
        );
        (uint8 v, bytes32 r, bytes32 s) = vm.sign(pk, digest);
        vault.joinWithPermit(who, path, amount, deadline, v, r, s);
    }

    function _lobby()
        internal
        view
        returns (uint256 entry, uint256 pot, uint256 seats, bool claimsStarted, bool opened)
    {
        (entry, pot, seats, claimsStarted, opened) = vault.lobbies(path);
    }
}
