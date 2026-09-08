// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {Test} from "forge-std/Test.sol";
import {Usdt} from "../src/Usdt.sol";
import {IPermit2, SwVault} from "../src/SwVault.sol";

contract MockPermit2 {
    function permitTransferFrom(
        IPermit2.PermitTransferFrom memory permit,
        IPermit2.SignatureTransferDetails calldata details,
        address owner,
        bytes calldata
    ) external {
        require(details.requestedAmount <= permit.permitted.amount, "amount");
        Usdt(permit.permitted.token).transferFrom(owner, details.to, details.requestedAmount);
    }
}

contract SwVaultTest is Test {
    Usdt internal usdt;
    SwVault internal vault;
    MockPermit2 internal mockPermit2;
    address internal platform = address(this);
    uint256 internal playerPk = 1;
    address internal player = vm.addr(playerPk);
    bytes32 internal path = keccak256("01abcdef");

    function setUp() public {
        usdt = new Usdt(platform);
        mockPermit2 = new MockPermit2();
        vault = new SwVault(address(usdt), platform, address(mockPermit2));
        usdt.mint(player, 50_000_000);
    }

    function testJoinLeave() public {
        _joinWithPermit(player, playerPk, 5_000_000);
        (uint256 entry, uint256 pot, uint256 seats,,) = _lobby();
        assertEq(entry, 5_000_000);
        assertEq(pot, 5_000_000);
        assertEq(seats, 1);
        assertEq(usdt.balanceOf(player), 45_000_000);

        vault.leaveSeat(player, path);
        (, pot, seats,,) = _lobby();
        assertEq(pot, 0);
        assertEq(seats, 0);
        assertEq(usdt.balanceOf(player), 50_000_000);
    }

    function testJoinWithPermit2() public {
        vm.prank(player);
        usdt.approve(address(mockPermit2), 5_000_000);
        vault.joinWithPermit2(player, path, 5_000_000, 0, block.timestamp + 1, "");
        assertEq(usdt.balanceOf(address(vault)), 5_000_000);
        assertEq(vault.seats(path, player), 5_000_000);
    }

    function testClaimSplits() public {
        _joinWithPermit(player, playerPk, 10_000_000);
        address dest = vm.addr(2);
        vault.claim(player, path, 10_000_000, dest, 5);
        assertEq(usdt.balanceOf(platform), 200_000);
        assertEq(usdt.balanceOf(dest), 500_000);
        assertEq(usdt.balanceOf(player), 49_300_000);
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
        assertEq(usdt.balanceOf(player), 50_000_000);
        assertEq(vault.seats(path, player), 0);
    }

    function testOnlyPlatform() public {
        vm.prank(player);
        vm.expectRevert(SwVault.NotPlatform.selector);
        vault.joinWithPermit(player, path, 1, block.timestamp + 1, 0, bytes32(0), bytes32(0));
    }

    function _joinWithPermit(address who, uint256 pk, uint256 amount) internal {
        uint256 nonce = usdt.nonces(who);
        uint256 deadline = block.timestamp + 1 hours;
        bytes32 digest = keccak256(
            abi.encodePacked(
                "\x19\x01",
                usdt.DOMAIN_SEPARATOR(),
                keccak256(abi.encode(usdt.PERMIT_TYPEHASH(), who, address(vault), amount, nonce, deadline))
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
