use anchor_lang::prelude::*;
use anchor_spl::token::{self,Mint,Token,TokenAccount,TransferChecked};
declare_id!("38P3f9cKZhKb2rPDrzb3DVAV4NRhZ1qGUGJetYiPpee4");
const BPS:u128=10_000;

#[program]
pub mod curve_protocol {
 use super::*;
 pub fn initialize(ctx:Context<Initialize>,quote_authority:Pubkey,treasury:Pubkey,pump_program:Pubkey,max_quote_age:u32)->Result<()>{
  require!(quote_authority!=Pubkey::default()&&treasury!=Pubkey::default()&&pump_program!=Pubkey::default(),ErrorCode::InvalidAuthority);
  require!((15..=300).contains(&max_quote_age),ErrorCode::InvalidQuoteAge);
  let c=&mut ctx.accounts.config;c.authority=ctx.accounts.authority.key();c.pending_authority=Pubkey::default();c.quote_authority=quote_authority;c.treasury=treasury;c.curve_mint=ctx.accounts.curve_mint.key();c.pump_program=pump_program;c.reward_vault=ctx.accounts.reward_vault.key();c.reward_bps=11_000;c.max_quote_age=max_quote_age;c.paused=true;c.bump=ctx.bumps.config;c.reward_bump=ctx.bumps.reward_vault;Ok(())
 }
 pub fn fund_rewards(ctx:Context<FundRewards>,amount:u64)->Result<()>{
  require!(amount>0,ErrorCode::ZeroAmount);token::transfer_checked(ctx.accounts.transfer_ctx(),amount,ctx.accounts.curve_mint.decimals)?;
  emit!(RewardsFunded{funder:ctx.accounts.funder.key(),amount});Ok(())
 }
 pub fn post_quote(ctx:Context<PostQuote>,a:PostQuoteArgs)->Result<()>{
  let c=&ctx.accounts.config;require!(!c.paused,ErrorCode::Paused);require!(a.deposit_amount>0&&a.base_curve_amount>0&&a.realizable_lamports>0,ErrorCode::ZeroAmount);
  require_keys_eq!(*ctx.accounts.bonding_curve.owner,c.pump_program,ErrorCode::InvalidCurve);
  let now=Clock::get()?.unix_timestamp;require!(a.expires_at>now&&a.expires_at<=now.checked_add(c.max_quote_age as i64).ok_or(ErrorCode::Math)?,ErrorCode::InvalidExpiry);
  let out=((a.base_curve_amount as u128).checked_mul(c.reward_bps as u128).ok_or(ErrorCode::Math)?.checked_div(BPS).ok_or(ErrorCode::Math)?) as u64;
  let q=&mut ctx.accounts.quote;q.depositor=ctx.accounts.depositor.key();q.token_mint=ctx.accounts.token_mint.key();q.bonding_curve=ctx.accounts.bonding_curve.key();q.deposit_amount=a.deposit_amount;q.realizable_lamports=a.realizable_lamports;q.base_curve_amount=a.base_curve_amount;q.final_curve_amount=out;q.nonce=a.nonce;q.posted_at=now;q.expires_at=a.expires_at;q.bump=ctx.bumps.quote;
  emit!(QuotePosted{quote:q.key(),depositor:q.depositor,token_mint:q.token_mint,final_curve_amount:out,expires_at:q.expires_at});Ok(())
 }
 pub fn absorb(ctx:Context<Absorb>,nonce:u64,minimum_output:u64)->Result<()>{
  let now=Clock::get()?.unix_timestamp;let c=&ctx.accounts.config;let q=&ctx.accounts.quote;
  require!(!c.paused,ErrorCode::Paused);require!(q.nonce==nonce&&q.depositor==ctx.accounts.depositor.key()&&q.token_mint==ctx.accounts.token_mint.key()&&q.bonding_curve==ctx.accounts.bonding_curve.key(),ErrorCode::QuoteMismatch);
  require!(now<=q.expires_at,ErrorCode::Expired);require_keys_eq!(*ctx.accounts.bonding_curve.owner,c.pump_program,ErrorCode::InvalidCurve);require!(q.final_curve_amount>=minimum_output,ErrorCode::Slippage);require!(ctx.accounts.reward_vault.amount>=q.final_curve_amount,ErrorCode::Reserve);
  token::transfer_checked(ctx.accounts.deposit_ctx(),q.deposit_amount,ctx.accounts.token_mint.decimals)?;
  let seeds:&[&[u8]]=&[b"config",&[c.bump]];token::transfer_checked(ctx.accounts.reward_ctx().with_signer(&[seeds]),q.final_curve_amount,ctx.accounts.curve_mint.decimals)?;
  let p=&mut ctx.accounts.position;if p.token_mint==Pubkey::default(){p.token_mint=ctx.accounts.token_mint.key();p.bonding_curve=ctx.accounts.bonding_curve.key();p.collateral_vault=ctx.accounts.collateral_vault.key();p.status=PositionStatus::OnCurve;p.created_at=now;p.bump=ctx.bumps.position;p.collateral_bump=ctx.bumps.collateral_vault;}
  require_keys_eq!(p.bonding_curve,ctx.accounts.bonding_curve.key(),ErrorCode::PositionMismatch);
  p.amount_absorbed=p.amount_absorbed.checked_add(q.deposit_amount).ok_or(ErrorCode::Math)?;p.curve_distributed=p.curve_distributed.checked_add(q.final_curve_amount).ok_or(ErrorCode::Math)?;p.realizable_lamports=p.realizable_lamports.checked_add(q.realizable_lamports).ok_or(ErrorCode::Math)?;p.absorption_count=p.absorption_count.checked_add(1).ok_or(ErrorCode::Math)?;p.updated_at=now;
  emit!(Absorbed{depositor:ctx.accounts.depositor.key(),token_mint:p.token_mint,deposit_amount:q.deposit_amount,curve_received:q.final_curve_amount});Ok(())
 }
 pub fn set_paused(ctx:Context<Admin>,paused:bool)->Result<()>{ctx.accounts.config.paused=paused;emit!(PauseChanged{paused});Ok(())}
 pub fn set_quote_authority(ctx:Context<Admin>,key:Pubkey)->Result<()>{require!(key!=Pubkey::default(),ErrorCode::InvalidAuthority);ctx.accounts.config.quote_authority=key;Ok(())}
 pub fn propose_authority(ctx:Context<Admin>,key:Pubkey)->Result<()>{require!(key!=Pubkey::default(),ErrorCode::InvalidAuthority);ctx.accounts.config.pending_authority=key;Ok(())}
 pub fn accept_authority(ctx:Context<AcceptAuthority>)->Result<()>{require_keys_eq!(ctx.accounts.config.pending_authority,ctx.accounts.pending.key(),ErrorCode::InvalidAuthority);ctx.accounts.config.authority=ctx.accounts.pending.key();ctx.accounts.config.pending_authority=Pubkey::default();Ok(())}
 pub fn mark_graduated(ctx:Context<MarkGraduated>)->Result<()>{require!(ctx.accounts.position.status==PositionStatus::OnCurve,ErrorCode::Status);ctx.accounts.position.status=PositionStatus::Graduated;ctx.accounts.position.updated_at=Clock::get()?.unix_timestamp;Ok(())}
}

#[derive(Accounts)]pub struct Initialize<'info>{#[account(mut)]pub authority:Signer<'info>,#[account(init,payer=authority,space=8+Config::INIT_SPACE,seeds=[b"config"],bump)]pub config:Box<Account<'info,Config>>,pub curve_mint:Box<Account<'info,Mint>>,#[account(init,payer=authority,token::mint=curve_mint,token::authority=config,seeds=[b"rewards"],bump)]pub reward_vault:Box<Account<'info,TokenAccount>>,pub system_program:Program<'info,System>,pub token_program:Program<'info,Token>,pub rent:Sysvar<'info,Rent>}
#[derive(Accounts)]pub struct FundRewards<'info>{#[account(mut)]pub funder:Signer<'info>,#[account(seeds=[b"config"],bump=config.bump,has_one=curve_mint,has_one=reward_vault)]pub config:Box<Account<'info,Config>>,pub curve_mint:Box<Account<'info,Mint>>,#[account(mut,constraint=funder_curve.mint==curve_mint.key(),constraint=funder_curve.owner==funder.key())]pub funder_curve:Box<Account<'info,TokenAccount>>,#[account(mut,seeds=[b"rewards"],bump=config.reward_bump)]pub reward_vault:Box<Account<'info,TokenAccount>>,pub token_program:Program<'info,Token>}
impl<'info> FundRewards<'info>{fn transfer_ctx(&self)->CpiContext<'_,'_,'_,'info,TransferChecked<'info>>{CpiContext::new(self.token_program.to_account_info(),TransferChecked{from:self.funder_curve.to_account_info(),mint:self.curve_mint.to_account_info(),to:self.reward_vault.to_account_info(),authority:self.funder.to_account_info()})}}
#[derive(AnchorSerialize,AnchorDeserialize,Clone)]pub struct PostQuoteArgs{pub nonce:u64,pub deposit_amount:u64,pub realizable_lamports:u64,pub base_curve_amount:u64,pub expires_at:i64}
#[derive(Accounts)]#[instruction(a:PostQuoteArgs)]pub struct PostQuote<'info>{#[account(mut)]pub quote_authority:Signer<'info>,#[account(seeds=[b"config"],bump=config.bump,has_one=quote_authority)]pub config:Box<Account<'info,Config>>,/// CHECK: bound into quote
pub depositor:UncheckedAccount<'info>,pub token_mint:Box<Account<'info,Mint>>,/// CHECK: owner verified
pub bonding_curve:UncheckedAccount<'info>,#[account(init,payer=quote_authority,space=8+Quote::INIT_SPACE,seeds=[b"quote",depositor.key().as_ref(),token_mint.key().as_ref(),&a.nonce.to_le_bytes()],bump)]pub quote:Box<Account<'info,Quote>>,pub system_program:Program<'info,System>}
#[derive(Accounts)]#[instruction(nonce:u64)]pub struct Absorb<'info>{#[account(mut)]pub depositor:Signer<'info>,#[account(seeds=[b"config"],bump=config.bump,has_one=curve_mint,has_one=reward_vault)]pub config:Box<Account<'info,Config>>,pub token_mint:Box<Account<'info,Mint>>,pub curve_mint:Box<Account<'info,Mint>>,/// CHECK: owner and address verified
pub bonding_curve:UncheckedAccount<'info>,#[account(mut,close=depositor,seeds=[b"quote",depositor.key().as_ref(),token_mint.key().as_ref(),&nonce.to_le_bytes()],bump=quote.bump)]pub quote:Box<Account<'info,Quote>>,#[account(init_if_needed,payer=depositor,space=8+Position::INIT_SPACE,seeds=[b"position",token_mint.key().as_ref()],bump)]pub position:Box<Account<'info,Position>>,#[account(init_if_needed,payer=depositor,token::mint=token_mint,token::authority=config,seeds=[b"collateral",token_mint.key().as_ref()],bump)]pub collateral_vault:Box<Account<'info,TokenAccount>>,#[account(mut,constraint=depositor_token.mint==token_mint.key(),constraint=depositor_token.owner==depositor.key())]pub depositor_token:Box<Account<'info,TokenAccount>>,#[account(mut,constraint=depositor_curve.mint==curve_mint.key(),constraint=depositor_curve.owner==depositor.key())]pub depositor_curve:Box<Account<'info,TokenAccount>>,#[account(mut,seeds=[b"rewards"],bump=config.reward_bump)]pub reward_vault:Box<Account<'info,TokenAccount>>,pub system_program:Program<'info,System>,pub token_program:Program<'info,Token>,pub rent:Sysvar<'info,Rent>}
impl<'info> Absorb<'info>{fn deposit_ctx(&self)->CpiContext<'_,'_,'_,'info,TransferChecked<'info>>{CpiContext::new(self.token_program.to_account_info(),TransferChecked{from:self.depositor_token.to_account_info(),mint:self.token_mint.to_account_info(),to:self.collateral_vault.to_account_info(),authority:self.depositor.to_account_info()})}fn reward_ctx(&self)->CpiContext<'_,'_,'_,'info,TransferChecked<'info>>{CpiContext::new(self.token_program.to_account_info(),TransferChecked{from:self.reward_vault.to_account_info(),mint:self.curve_mint.to_account_info(),to:self.depositor_curve.to_account_info(),authority:self.config.to_account_info()})}}
#[derive(Accounts)]pub struct Admin<'info>{pub authority:Signer<'info>,#[account(mut,seeds=[b"config"],bump=config.bump,has_one=authority)]pub config:Box<Account<'info,Config>>}
#[derive(Accounts)]pub struct AcceptAuthority<'info>{pub pending:Signer<'info>,#[account(mut,seeds=[b"config"],bump=config.bump)]pub config:Box<Account<'info,Config>>}
#[derive(Accounts)]pub struct MarkGraduated<'info>{pub quote_authority:Signer<'info>,#[account(seeds=[b"config"],bump=config.bump,has_one=quote_authority)]pub config:Box<Account<'info,Config>>,#[account(mut,seeds=[b"position",position.token_mint.as_ref()],bump=position.bump)]pub position:Box<Account<'info,Position>>}

#[account]#[derive(InitSpace)]pub struct Config{pub authority:Pubkey,pub pending_authority:Pubkey,pub quote_authority:Pubkey,pub treasury:Pubkey,pub curve_mint:Pubkey,pub pump_program:Pubkey,pub reward_vault:Pubkey,pub reward_bps:u16,pub max_quote_age:u32,pub paused:bool,pub bump:u8,pub reward_bump:u8}
#[account]#[derive(InitSpace)]pub struct Quote{pub depositor:Pubkey,pub token_mint:Pubkey,pub bonding_curve:Pubkey,pub deposit_amount:u64,pub realizable_lamports:u64,pub base_curve_amount:u64,pub final_curve_amount:u64,pub nonce:u64,pub posted_at:i64,pub expires_at:i64,pub bump:u8}
#[account]#[derive(InitSpace)]pub struct Position{pub token_mint:Pubkey,pub bonding_curve:Pubkey,pub collateral_vault:Pubkey,pub amount_absorbed:u64,pub curve_distributed:u64,pub realizable_lamports:u64,pub absorption_count:u64,pub status:PositionStatus,pub created_at:i64,pub updated_at:i64,pub bump:u8,pub collateral_bump:u8}
#[derive(AnchorSerialize,AnchorDeserialize,Clone,Copy,PartialEq,Eq,InitSpace)]pub enum PositionStatus{OnCurve,Graduated,HarvestReady,Harvested,DeadCurve}
#[event]pub struct RewardsFunded{pub funder:Pubkey,pub amount:u64}#[event]pub struct QuotePosted{pub quote:Pubkey,pub depositor:Pubkey,pub token_mint:Pubkey,pub final_curve_amount:u64,pub expires_at:i64}#[event]pub struct Absorbed{pub depositor:Pubkey,pub token_mint:Pubkey,pub deposit_amount:u64,pub curve_received:u64}#[event]pub struct PauseChanged{pub paused:bool}
#[error_code]pub enum ErrorCode{#[msg("Invalid authority or protocol address")]InvalidAuthority,#[msg("Quote age must be 15-300 seconds")]InvalidQuoteAge,#[msg("Amount must be nonzero")]ZeroAmount,#[msg("Deposits are paused")]Paused,#[msg("Invalid Pump.fun bonding curve")]InvalidCurve,#[msg("Invalid quote expiry")]InvalidExpiry,#[msg("Arithmetic overflow")]Math,#[msg("Quote does not match")]QuoteMismatch,#[msg("Quote expired")]Expired,#[msg("Minimum output not met")]Slippage,#[msg("Insufficient reward reserve")]Reserve,#[msg("Position mismatch")]PositionMismatch,#[msg("Invalid position status")]Status}
