use anchor_lang::prelude::*;
use anchor_spl::associated_token::AssociatedToken;
use anchor_spl::token::{self, Mint, Token, TokenAccount, TransferChecked};

declare_id!("8NZHj9VH9JkqiAg19CK43ZLuK5hn5jXPBnLfbeKonqfy");

const PLATFORM_FEE_PCT: u64 = 2;
/// Same cap as `vault_oracle::split_pot`.
const MAX_DEV_FEE_PCT: u8 = 5;

#[program]
pub mod sw_vault {
    use super::*;

    /// One-time setup. The deployer (`payer`) is the platform: fee payer,
    /// remaining signer, and 2% claim recipient.
    pub fn initialize(ctx: Context<Initialize>) -> Result<()> {
        let config = &mut ctx.accounts.config;
        config.platform = ctx.accounts.payer.key();
        config.usdc_mint = ctx.accounts.usdc_mint.key();
        config.bump = ctx.bumps.config;
        Ok(())
    }

    pub fn join(ctx: Context<Join>, path_hash: [u8; 32], amount: u64) -> Result<()> {
        require!(amount > 0, VaultError::ZeroAmount);
        require_keys_eq!(
            ctx.accounts.config.usdc_mint,
            ctx.accounts.usdc_mint.key(),
            VaultError::WrongMint
        );

        let escrow = &mut ctx.accounts.escrow;
        if escrow.path_hash == [0u8; 32] {
            escrow.path_hash = path_hash;
            escrow.entry = amount;
            escrow.bump = ctx.bumps.escrow;
            escrow.pot = 0;
            escrow.seats = 0;
            escrow.claims_started = false;
        } else {
            require!(escrow.path_hash == path_hash, VaultError::PathMismatch);
            require!(amount == escrow.entry, VaultError::EntryMismatch);
            require!(!escrow.claims_started, VaultError::ClaimsStarted);
        }

        let seat = &mut ctx.accounts.seat;
        require!(seat.amount == 0, VaultError::AlreadySeated);
        seat.player = ctx.accounts.player.key();
        seat.path_hash = path_hash;
        seat.amount = amount;
        seat.bump = ctx.bumps.seat;

        token::transfer_checked(
            CpiContext::new(
                ctx.accounts.token_program.key(),
                TransferChecked {
                    from: ctx.accounts.player_usdc.to_account_info(),
                    mint: ctx.accounts.usdc_mint.to_account_info(),
                    to: ctx.accounts.vault_usdc.to_account_info(),
                    authority: ctx.accounts.player.to_account_info(),
                },
            ),
            amount,
            ctx.accounts.usdc_mint.decimals,
        )?;

        escrow.pot = escrow
            .pot
            .checked_add(amount)
            .ok_or(VaultError::Overflow)?;
        escrow.seats = escrow
            .seats
            .checked_add(1)
            .ok_or(VaultError::Overflow)?;
        Ok(())
    }

    pub fn leave(ctx: Context<Leave>) -> Result<()> {
        refund(
            &ctx.accounts.config,
            &mut ctx.accounts.escrow,
            &ctx.accounts.seat,
            &ctx.accounts.player_usdc,
            &ctx.accounts.vault_usdc,
            &ctx.accounts.usdc_mint,
            &ctx.accounts.token_program,
        )
    }

    pub fn kick(ctx: Context<Kick>) -> Result<()> {
        refund(
            &ctx.accounts.config,
            &mut ctx.accounts.escrow,
            &ctx.accounts.seat,
            &ctx.accounts.player_usdc,
            &ctx.accounts.vault_usdc,
            &ctx.accounts.usdc_mint,
            &ctx.accounts.token_program,
        )
    }

    /// Split is on-chain: 2% platform, `dev_fee`% (0–5) to the game, rest winner.
    pub fn claim(ctx: Context<Claim>, amount: u64, dev_fee: u8) -> Result<()> {
        require!(amount > 0, VaultError::ZeroAmount);
        require!(
            ctx.accounts.escrow.pot >= amount,
            VaultError::InsufficientPot
        );
        let (winner_amt, platform_amt, dev_amt) = split(amount, dev_fee)?;

        let bump = ctx.accounts.escrow.bump;
        let path_hash = ctx.accounts.escrow.path_hash;
        let seeds: &[&[u8]] = &[b"lobby", path_hash.as_ref(), &[bump]];
        let signer = &[seeds];
        let decimals = ctx.accounts.usdc_mint.decimals;
        let token_program = ctx.accounts.token_program.key();
        let vault = ctx.accounts.vault_usdc.to_account_info();
        let mint = ctx.accounts.usdc_mint.to_account_info();
        let escrow_ai = ctx.accounts.escrow.to_account_info();

        if winner_amt > 0 {
            token::transfer_checked(
                CpiContext::new_with_signer(
                    token_program.clone(),
                    TransferChecked {
                        from: vault.clone(),
                        mint: mint.clone(),
                        to: ctx.accounts.player_usdc.to_account_info(),
                        authority: escrow_ai.clone(),
                    },
                    signer,
                ),
                winner_amt,
                decimals,
            )?;
        }
        if platform_amt > 0 {
            token::transfer_checked(
                CpiContext::new_with_signer(
                    token_program.clone(),
                    TransferChecked {
                        from: vault.clone(),
                        mint: mint.clone(),
                        to: ctx.accounts.platform_usdc.to_account_info(),
                        authority: escrow_ai.clone(),
                    },
                    signer,
                ),
                platform_amt,
                decimals,
            )?;
        }
        if dev_amt > 0 {
            token::transfer_checked(
                CpiContext::new_with_signer(
                    token_program,
                    TransferChecked {
                        from: vault,
                        mint,
                        to: ctx.accounts.dev_usdc.to_account_info(),
                        authority: escrow_ai,
                    },
                    signer,
                ),
                dev_amt,
                decimals,
            )?;
        }

        ctx.accounts.escrow.pot = ctx
            .accounts
            .escrow
            .pot
            .checked_sub(amount)
            .ok_or(VaultError::Overflow)?;
        ctx.accounts.escrow.claims_started = true;
        Ok(())
    }
}

pub fn path_hash(path: &str) -> [u8; 32] {
    solana_sha256_hasher::hash(path.as_bytes()).to_bytes()
}

fn split(amount: u64, dev_fee: u8) -> Result<(u64, u64, u64)> {
    require!(dev_fee <= MAX_DEV_FEE_PCT, VaultError::DevFeeTooHigh);
    let platform = amount
        .checked_mul(PLATFORM_FEE_PCT)
        .ok_or(VaultError::Overflow)?
        / 100;
    let dev = if dev_fee == 0 {
        0
    } else {
        amount
            .checked_mul(dev_fee as u64)
            .ok_or(VaultError::Overflow)?
            / 100
    };
    let fees = platform
        .checked_add(dev)
        .ok_or(VaultError::Overflow)?;
    require!(fees <= amount, VaultError::InvalidSplit);
    Ok((amount - fees, platform, dev))
}

fn refund<'info>(
    config: &Account<'info, Config>,
    escrow: &mut Account<'info, LobbyEscrow>,
    seat: &Account<'info, Seat>,
    player_usdc: &Account<'info, TokenAccount>,
    vault_usdc: &Account<'info, TokenAccount>,
    mint: &Account<'info, Mint>,
    token_program: &Program<'info, Token>,
) -> Result<()> {
    require!(!escrow.claims_started, VaultError::ClaimsStarted);
    let amount = seat.amount;
    require!(amount > 0, VaultError::EmptySeat);
    let bump = escrow.bump;
    let path_hash = escrow.path_hash;
    let seeds: &[&[u8]] = &[b"lobby", path_hash.as_ref(), &[bump]];
    token::transfer_checked(
        CpiContext::new_with_signer(
            token_program.key(),
            TransferChecked {
                from: vault_usdc.to_account_info(),
                mint: mint.to_account_info(),
                to: player_usdc.to_account_info(),
                authority: escrow.to_account_info(),
            },
            &[&seeds],
        ),
        amount,
        mint.decimals,
    )?;
    require_keys_eq!(mint.key(), config.usdc_mint, VaultError::WrongMint);
    escrow.pot = escrow
        .pot
        .checked_sub(amount)
        .ok_or(VaultError::Overflow)?;
    escrow.seats = escrow.seats.saturating_sub(1);
    Ok(())
}

#[derive(Accounts)]
pub struct Initialize<'info> {
    #[account(mut)]
    pub payer: Signer<'info>,
    #[account(
        init,
        payer = payer,
        space = 8 + Config::INIT_SPACE,
        seeds = [b"config"],
        bump
    )]
    pub config: Account<'info, Config>,
    pub usdc_mint: Account<'info, Mint>,
    #[account(
        init_if_needed,
        payer = payer,
        associated_token::mint = usdc_mint,
        associated_token::authority = payer
    )]
    pub platform_usdc: Account<'info, TokenAccount>,
    pub token_program: Program<'info, Token>,
    pub associated_token_program: Program<'info, AssociatedToken>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
#[instruction(path_hash: [u8; 32])]
pub struct Join<'info> {
    /// Platform covers rent + tx fees so the player never needs SOL.
    #[account(mut, address = config.platform)]
    pub payer: Signer<'info>,
    pub player: Signer<'info>,
    #[account(seeds = [b"config"], bump = config.bump)]
    pub config: Account<'info, Config>,
    pub usdc_mint: Account<'info, Mint>,
    #[account(
        init_if_needed,
        payer = payer,
        space = 8 + LobbyEscrow::INIT_SPACE,
        seeds = [b"lobby", path_hash.as_ref()],
        bump,
        constraint = escrow.path_hash == [0u8; 32] || escrow.path_hash == path_hash @ VaultError::PathMismatch,
        constraint = !escrow.claims_started @ VaultError::ClaimsStarted
    )]
    pub escrow: Account<'info, LobbyEscrow>,
    #[account(
        init,
        payer = payer,
        space = 8 + Seat::INIT_SPACE,
        seeds = [b"seat", path_hash.as_ref(), player.key().as_ref()],
        bump
    )]
    pub seat: Account<'info, Seat>,
    #[account(
        mut,
        associated_token::mint = usdc_mint,
        associated_token::authority = player
    )]
    pub player_usdc: Account<'info, TokenAccount>,
    #[account(
        init_if_needed,
        payer = payer,
        associated_token::mint = usdc_mint,
        associated_token::authority = escrow
    )]
    pub vault_usdc: Account<'info, TokenAccount>,
    pub token_program: Program<'info, Token>,
    pub associated_token_program: Program<'info, AssociatedToken>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct Leave<'info> {
    #[account(mut, address = config.platform)]
    pub platform: Signer<'info>,
    /// CHECK: credited USDC destination; seat PDA binds the player.
    #[account(mut)]
    pub player: UncheckedAccount<'info>,
    #[account(seeds = [b"config"], bump = config.bump)]
    pub config: Account<'info, Config>,
    #[account(address = config.usdc_mint)]
    pub usdc_mint: Account<'info, Mint>,
    #[account(
        mut,
        seeds = [b"lobby", escrow.path_hash.as_ref()],
        bump = escrow.bump,
        constraint = !escrow.claims_started @ VaultError::ClaimsStarted
    )]
    pub escrow: Account<'info, LobbyEscrow>,
    #[account(
        mut,
        close = player,
        seeds = [b"seat", escrow.path_hash.as_ref(), player.key().as_ref()],
        bump = seat.bump,
        has_one = player
    )]
    pub seat: Account<'info, Seat>,
    #[account(
        mut,
        associated_token::mint = config.usdc_mint,
        associated_token::authority = player
    )]
    pub player_usdc: Account<'info, TokenAccount>,
    #[account(
        mut,
        associated_token::mint = config.usdc_mint,
        associated_token::authority = escrow
    )]
    pub vault_usdc: Account<'info, TokenAccount>,
    pub token_program: Program<'info, Token>,
}

#[derive(Accounts)]
pub struct Kick<'info> {
    #[account(mut, address = config.platform)]
    pub platform: Signer<'info>,
    /// CHECK: refund destination; seat PDA binds the player.
    #[account(mut)]
    pub player: UncheckedAccount<'info>,
    #[account(seeds = [b"config"], bump = config.bump)]
    pub config: Account<'info, Config>,
    #[account(address = config.usdc_mint)]
    pub usdc_mint: Account<'info, Mint>,
    #[account(
        mut,
        seeds = [b"lobby", escrow.path_hash.as_ref()],
        bump = escrow.bump,
        constraint = !escrow.claims_started @ VaultError::ClaimsStarted
    )]
    pub escrow: Account<'info, LobbyEscrow>,
    #[account(
        mut,
        close = player,
        seeds = [b"seat", escrow.path_hash.as_ref(), player.key().as_ref()],
        bump = seat.bump,
        has_one = player
    )]
    pub seat: Account<'info, Seat>,
    #[account(
        mut,
        associated_token::mint = config.usdc_mint,
        associated_token::authority = player
    )]
    pub player_usdc: Account<'info, TokenAccount>,
    #[account(
        mut,
        associated_token::mint = config.usdc_mint,
        associated_token::authority = escrow
    )]
    pub vault_usdc: Account<'info, TokenAccount>,
    pub token_program: Program<'info, Token>,
}

#[derive(Accounts)]
pub struct Claim<'info> {
    #[account(mut, address = config.platform)]
    pub platform: Signer<'info>,
    /// CHECK: winner USDC owner.
    pub player: UncheckedAccount<'info>,
    /// CHECK: optional game-dev USDC owner. Unused on-chain when `dev_fee` is 0.
    pub dev: UncheckedAccount<'info>,
    #[account(seeds = [b"config"], bump = config.bump)]
    pub config: Account<'info, Config>,
    #[account(address = config.usdc_mint)]
    pub usdc_mint: Account<'info, Mint>,
    #[account(
        mut,
        seeds = [b"lobby", escrow.path_hash.as_ref()],
        bump = escrow.bump
    )]
    pub escrow: Account<'info, LobbyEscrow>,
    #[account(
        mut,
        associated_token::mint = config.usdc_mint,
        associated_token::authority = player
    )]
    pub player_usdc: Box<Account<'info, TokenAccount>>,
    #[account(
        mut,
        associated_token::mint = config.usdc_mint,
        associated_token::authority = platform
    )]
    pub platform_usdc: Box<Account<'info, TokenAccount>>,
    #[account(
        mut,
        associated_token::mint = config.usdc_mint,
        associated_token::authority = dev
    )]
    pub dev_usdc: Box<Account<'info, TokenAccount>>,
    #[account(
        mut,
        associated_token::mint = config.usdc_mint,
        associated_token::authority = escrow
    )]
    pub vault_usdc: Box<Account<'info, TokenAccount>>,
    pub token_program: Program<'info, Token>,
}

#[account]
#[derive(InitSpace)]
pub struct Config {
    pub platform: Pubkey,
    pub usdc_mint: Pubkey,
    pub bump: u8,
}

#[account]
#[derive(InitSpace)]
pub struct LobbyEscrow {
    pub path_hash: [u8; 32],
    pub entry: u64,
    pub pot: u64,
    pub seats: u32,
    pub claims_started: bool,
    pub bump: u8,
}

#[account]
#[derive(InitSpace)]
pub struct Seat {
    pub player: Pubkey,
    pub path_hash: [u8; 32],
    pub amount: u64,
    pub bump: u8,
}

#[error_code]
pub enum VaultError {
    #[msg("amount must be greater than zero")]
    ZeroAmount,
    #[msg("player already has a seat in this lobby")]
    AlreadySeated,
    #[msg("seat is empty")]
    EmptySeat,
    #[msg("arithmetic overflow")]
    Overflow,
    #[msg("USDC mint does not match config")]
    WrongMint,
    #[msg("escrow pot is smaller than the claim")]
    InsufficientPot,
    #[msg("lobby path does not match escrow")]
    PathMismatch,
    #[msg("entry amount does not match this lobby")]
    EntryMismatch,
    #[msg("claims have started; join/leave/kick are frozen")]
    ClaimsStarted,
    #[msg("dev fee exceeds 5 percent")]
    DevFeeTooHigh,
    #[msg("platform plus dev fee exceeds claim amount")]
    InvalidSplit,
}
