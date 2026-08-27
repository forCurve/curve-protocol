import * as anchor from "@coral-xyz/anchor";
import { Program } from "@coral-xyz/anchor";
import { Keypair, PublicKey, SystemProgram, Transaction, sendAndConfirmTransaction } from "@solana/web3.js";
import { TOKEN_PROGRAM_ID, createMint, getAccount, getOrCreateAssociatedTokenAccount, mintTo } from "@solana/spl-token";
import { expect } from "chai";
import { CurveProtocol } from "../target/types/curve_protocol";

describe("curve-protocol", () => {
  anchor.setProvider(anchor.AnchorProvider.env());
  const provider = anchor.getProvider() as anchor.AnchorProvider;
  const program = anchor.workspace.CurveProtocol as Program<CurveProtocol>;
  const payer = (provider.wallet as anchor.Wallet & { payer: Keypair }).payer;
  const connection = provider.connection;

  let curveMint: PublicKey;
  let depositMint: PublicKey;
  let rewardVault: PublicKey;
  let config: PublicKey;
  let bondingCurve: Keypair;
  let pumpProgram: PublicKey;
  const depositor = Keypair.generate();

  before(async () => {
    [config] = PublicKey.findProgramAddressSync([Buffer.from("config")], program.programId);
    [rewardVault] = PublicKey.findProgramAddressSync([Buffer.from("rewards")], program.programId);
    curveMint = await createMint(connection, payer, payer.publicKey, null, 6);
    depositMint = await createMint(connection, payer, payer.publicKey, null, 6);
    pumpProgram = Keypair.generate().publicKey;
    bondingCurve = Keypair.generate();

    const lamports = await connection.getMinimumBalanceForRentExemption(64);
    await sendAndConfirmTransaction(connection, new Transaction().add(
      SystemProgram.createAccount({
        fromPubkey: payer.publicKey,
        newAccountPubkey: bondingCurve.publicKey,
        lamports,
        space: 64,
        programId: pumpProgram,
      })
    ), [payer, bondingCurve]);

    await program.methods.initialize(payer.publicKey, payer.publicKey, pumpProgram, 60)
      .accountsStrict({
        authority: payer.publicKey,
        config,
        curveMint,
        rewardVault,
        systemProgram: SystemProgram.programId,
        tokenProgram: TOKEN_PROGRAM_ID,
        rent: anchor.web3.SYSVAR_RENT_PUBKEY,
      }).rpc();

    const funderCurve = await getOrCreateAssociatedTokenAccount(connection, payer, curveMint, payer.publicKey);
    await mintTo(connection, payer, curveMint, funderCurve.address, payer, 10_000_000);
    await program.methods.fundRewards(new anchor.BN(10_000_000)).accountsStrict({
      funder: payer.publicKey, config, curveMint, funderCurve: funderCurve.address, rewardVault, tokenProgram: TOKEN_PROGRAM_ID,
    }).rpc();

    await program.methods.setPaused(false).accountsStrict({ authority: payer.publicKey, config }).rpc();

    const sig = await connection.requestAirdrop(depositor.publicKey, 5 * anchor.web3.LAMPORTS_PER_SOL);
    await connection.confirmTransaction(sig, "confirmed");
  });

  it("initializes paused with a finite funded reserve", async () => {
    const state = await program.account.config.fetch(config);
    expect(state.paused).eq(false);
    expect(state.rewardBps).eq(11_000);
    expect(state.curveMint.toBase58()).eq(curveMint.toBase58());
    expect((await getAccount(connection, rewardVault)).amount).eq(10_000_000n);
  });

  it("rejects unauthorized administrative changes", async () => {
    try {
      await program.methods.setPaused(true).accountsStrict({ authority: depositor.publicKey, config }).signers([depositor]).rpc();
      expect.fail("unauthorized pause unexpectedly succeeded");
    } catch (error) {
      expect(String(error)).to.contain("ConstraintHasOne");
    }
  });

  it("absorbs collateral and releases exactly 110% from the reserve", async () => {
    const userDeposit = await getOrCreateAssociatedTokenAccount(connection, payer, depositMint, depositor.publicKey);
    const userCurve = await getOrCreateAssociatedTokenAccount(connection, payer, curveMint, depositor.publicKey);
    await mintTo(connection, payer, depositMint, userDeposit.address, payer, 2_000_000);

    const nonce = new anchor.BN(7);
    const nonceSeed = nonce.toArrayLike(Buffer, "le", 8);
    const [quote] = PublicKey.findProgramAddressSync([Buffer.from("quote"), depositor.publicKey.toBuffer(), depositMint.toBuffer(), nonceSeed], program.programId);
    const [position] = PublicKey.findProgramAddressSync([Buffer.from("position"), depositMint.toBuffer()], program.programId);
    const [collateralVault] = PublicKey.findProgramAddressSync([Buffer.from("collateral"), depositMint.toBuffer()], program.programId);
    const now = Math.floor(Date.now() / 1000);

    await program.methods.postQuote({
      nonce,
      depositAmount: new anchor.BN(1_000_000),
      realizableLamports: new anchor.BN(25_000_000),
      baseCurveAmount: new anchor.BN(1_000_000),
      expiresAt: new anchor.BN(now + 45),
    }).accountsStrict({
      quoteAuthority: payer.publicKey, config, depositor: depositor.publicKey, tokenMint: depositMint,
      bondingCurve: bondingCurve.publicKey, quote, systemProgram: SystemProgram.programId,
    }).rpc();

    await program.methods.absorb(nonce, new anchor.BN(1_090_000)).accountsStrict({
      depositor: depositor.publicKey, config, tokenMint: depositMint, curveMint, bondingCurve: bondingCurve.publicKey,
      quote, position, collateralVault, depositorToken: userDeposit.address, depositorCurve: userCurve.address,
      rewardVault, systemProgram: SystemProgram.programId, tokenProgram: TOKEN_PROGRAM_ID, rent: anchor.web3.SYSVAR_RENT_PUBKEY,
    }).signers([depositor]).rpc();

    expect((await getAccount(connection, collateralVault)).amount).eq(1_000_000n);
    expect((await getAccount(connection, userCurve.address)).amount).eq(1_100_000n);
    expect((await getAccount(connection, rewardVault)).amount).eq(8_900_000n);
    const p = await program.account.position.fetch(position);
    expect(p.amountAbsorbed.toString()).eq("1000000");
    expect(p.curveDistributed.toString()).eq("1100000");
  });

  it("rejects quotes after deposits are paused", async () => {
    await program.methods.setPaused(true).accountsStrict({ authority: payer.publicKey, config }).rpc();
    const nonce = new anchor.BN(8);
    const [quote] = PublicKey.findProgramAddressSync([Buffer.from("quote"), depositor.publicKey.toBuffer(), depositMint.toBuffer(), nonce.toArrayLike(Buffer, "le", 8)], program.programId);
    try {
      await program.methods.postQuote({
        nonce, depositAmount: new anchor.BN(1), realizableLamports: new anchor.BN(1),
        baseCurveAmount: new anchor.BN(1), expiresAt: new anchor.BN(Math.floor(Date.now()/1000)+30),
      }).accountsStrict({
        quoteAuthority: payer.publicKey, config, depositor: depositor.publicKey, tokenMint: depositMint,
        bondingCurve: bondingCurve.publicKey, quote, systemProgram: SystemProgram.programId,
      }).rpc();
      expect.fail("paused quote unexpectedly succeeded");
    } catch (error) {
      expect(String(error)).to.contain("Deposits are paused");
    }
  });
});
