// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

interface IUsdt {
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

interface IPermit2 {
    struct TokenPermissions {
        address token;
        uint256 amount;
    }

    struct PermitTransferFrom {
        TokenPermissions permitted;
        uint256 nonce;
        uint256 deadline;
    }

    struct SignatureTransferDetails {
        address to;
        uint256 requestedAmount;
    }

    function permitTransferFrom(
        PermitTransferFrom memory permit,
        SignatureTransferDetails calldata transferDetails,
        address owner,
        bytes calldata signature
    ) external;
}

/// Lobby escrow. Platform is the only msg.sender.
/// Dest play USDT uses ERC-2612 `joinWithPermit`.
/// Mainnet USDT (no permit) uses Permit2 `joinWithPermit2`.
contract SwVault {
    uint256 public constant PLATFORM_FEE_PCT = 2;
    uint256 public constant MAX_DEST_FEE_PCT = 5;

    IUsdt public immutable usdt;
    IPermit2 public immutable permit2;
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

    constructor(address usdt_, address platform_, address permit2_) {
        usdt = IUsdt(usdt_);
        platform = platform_;
        permit2 = IPermit2(permit2_);
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
        usdt.permit(player, address(this), amount, deadline, v, r, s);
        _pullAndSeat(player, lobbyPathHash, amount);
    }

    function joinWithPermit2(
        address player,
        bytes32 lobbyPathHash,
        uint256 amount,
        uint256 nonce,
        uint256 deadline,
        bytes calldata signature
    ) external onlyPlatform {
        permit2.permitTransferFrom(
            IPermit2.PermitTransferFrom({
                permitted: IPermit2.TokenPermissions({token: address(usdt), amount: amount}),
                nonce: nonce,
                deadline: deadline
            }),
            IPermit2.SignatureTransferDetails({to: address(this), requestedAmount: amount}),
            player,
            signature
        );
        _seat(player, lobbyPathHash, amount);
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
            usdt.transfer(player, winnerAmt);
        }
        if (platformAmt > 0) {
            usdt.transfer(platform, platformAmt);
        }
        if (destAmt > 0) {
            usdt.transfer(dest, destAmt);
        }

        lobby.pot -= amount;
        lobby.claimsStarted = true;
        emit Claimed(lobbyPathHash, player, amount, dest, destFeePct);
    }

    function _pullAndSeat(address player, bytes32 lobbyPathHash, uint256 amount) internal {
        _validateJoin(player, lobbyPathHash, amount);
        usdt.transferFrom(player, address(this), amount);
        _recordSeat(player, lobbyPathHash, amount);
    }

    function _seat(address player, bytes32 lobbyPathHash, uint256 amount) internal {
        _validateJoin(player, lobbyPathHash, amount);
        _recordSeat(player, lobbyPathHash, amount);
    }

    function _validateJoin(address player, bytes32 lobbyPathHash, uint256 amount) internal view {
        if (amount == 0) revert ZeroAmount();
        Lobby storage lobby = lobbies[lobbyPathHash];
        if (lobby.opened) {
            if (amount != lobby.entry) revert EntryMismatch();
            if (lobby.claimsStarted) revert ClaimsStarted();
        }
        if (seats[lobbyPathHash][player] != 0) revert AlreadySeated();
    }

    function _recordSeat(address player, bytes32 lobbyPathHash, uint256 amount) internal {
        Lobby storage lobby = lobbies[lobbyPathHash];
        if (!lobby.opened) {
            lobby.opened = true;
            lobby.entry = amount;
        }
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
        usdt.transfer(player, amount);
    }
}
