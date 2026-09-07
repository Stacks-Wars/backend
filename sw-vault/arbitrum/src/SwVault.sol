// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

interface IUsdCx {
    function transferFrom(address from, address to, uint256 amount) external returns (bool);
    function transfer(address to, uint256 amount) external returns (bool);
    function permit(
        address owner,
        address spender,
        uint256 value,
        uint256 deadline,
        uint8 v,
        bytes32 r,
        bytes32 s
    ) external;
}

/// Lobby escrow. Platform is the only msg.sender; join is permit-only.
contract SwVault {
    uint256 public constant PLATFORM_FEE_PCT = 2;
    uint256 public constant MAX_DEST_FEE_PCT = 5;

    IUsdCx public immutable usdc;
    address public immutable platform;

    struct Lobby {
        uint256 entry;
        uint256 pot;
        uint256 seats;
        bool claimsStarted;
        bool opened;
    }

    mapping(bytes32 => Lobby) public lobbies;
    mapping(bytes32 => mapping(address => uint256)) public seats;

    event Joined(bytes32 indexed pathHash, address indexed player, uint256 amount);
    event Left(bytes32 indexed pathHash, address indexed player, uint256 amount);
    event Kicked(bytes32 indexed pathHash, address indexed player, uint256 amount);
    event Claimed(bytes32 indexed pathHash, address indexed player, uint256 amount, address dest, uint256 destFeePct);

    error NotPlatform();
    error ZeroAmount();
    error AlreadySeated();
    error NotSeated();
    error EntryMismatch();
    error ClaimsStarted();
    error InsufficientPot();
    error DestFeeTooHigh();

    modifier onlyPlatform() {
        if (msg.sender != platform) revert NotPlatform();
        _;
    }

    constructor(address usdc_, address platform_) {
        usdc = IUsdCx(usdc_);
        platform = platform_;
    }

    function pathHash(string calldata path) external pure returns (bytes32) {
        return keccak256(bytes(path));
    }

    function joinWithPermit(
        address player,
        bytes32 lobbyPathHash,
        uint256 amount,
        uint256 deadline,
        uint8 v,
        bytes32 r,
        bytes32 s
    ) external onlyPlatform {
        usdc.permit(player, address(this), amount, deadline, v, r, s);
        _join(player, lobbyPathHash, amount);
    }

    function leaveSeat(address player, bytes32 lobbyPathHash) external onlyPlatform {
        uint256 amount = _refund(player, lobbyPathHash);
        emit Left(lobbyPathHash, player, amount);
    }

    function kick(address player, bytes32 lobbyPathHash) external onlyPlatform {
        uint256 amount = _refund(player, lobbyPathHash);
        emit Kicked(lobbyPathHash, player, amount);
    }

    function claim(address player, bytes32 lobbyPathHash, uint256 amount, address dest, uint8 destFeePct)
        external
        onlyPlatform
    {
        if (amount == 0) revert ZeroAmount();
        if (destFeePct > MAX_DEST_FEE_PCT) revert DestFeeTooHigh();
        Lobby storage lobby = lobbies[lobbyPathHash];
        if (seats[lobbyPathHash][player] == 0) revert NotSeated();
        if (lobby.pot < amount) revert InsufficientPot();

        uint256 platformAmt = (amount * PLATFORM_FEE_PCT) / 100;
        uint256 destAmt = destFeePct == 0 || dest == address(0) || dest == player || dest == platform
            ? 0
            : (amount * destFeePct) / 100;
        uint256 winnerAmt = amount - platformAmt - destAmt;

        if (winnerAmt > 0) {
            usdc.transfer(player, winnerAmt);
        }
        if (platformAmt > 0) {
            usdc.transfer(platform, platformAmt);
        }
        if (destAmt > 0) {
            usdc.transfer(dest, destAmt);
        }

        lobby.pot -= amount;
        lobby.claimsStarted = true;
        emit Claimed(lobbyPathHash, player, amount, dest, destFeePct);
    }

    function _join(address player, bytes32 lobbyPathHash, uint256 amount) internal {
        if (amount == 0) revert ZeroAmount();
        Lobby storage lobby = lobbies[lobbyPathHash];
        if (!lobby.opened) {
            lobby.opened = true;
            lobby.entry = amount;
        } else {
            if (amount != lobby.entry) revert EntryMismatch();
            if (lobby.claimsStarted) revert ClaimsStarted();
        }
        if (seats[lobbyPathHash][player] != 0) revert AlreadySeated();

        usdc.transferFrom(player, address(this), amount);
        seats[lobbyPathHash][player] = amount;
        lobby.pot += amount;
        lobby.seats += 1;
        emit Joined(lobbyPathHash, player, amount);
    }

    function _refund(address player, bytes32 lobbyPathHash) internal returns (uint256 amount) {
        Lobby storage lobby = lobbies[lobbyPathHash];
        if (lobby.claimsStarted) revert ClaimsStarted();
        amount = seats[lobbyPathHash][player];
        if (amount == 0) revert NotSeated();
        seats[lobbyPathHash][player] = 0;
        lobby.pot -= amount;
        lobby.seats -= 1;
        usdc.transfer(player, amount);
    }
}
