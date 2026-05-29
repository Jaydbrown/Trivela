import {
  Address,
  Contract,
  TransactionBuilder,
  BASE_FEE,
  scValToNative,
  nativeToScVal,
  rpc,
  Transaction,
} from '@stellar/stellar-sdk';

export interface ClientOptions {
  rpcUrl: string;
  networkPassphrase: string;
  contractId: string;
  allowHttp?: boolean;
  publicKey?: string;
  signTransaction?: (tx: string, opts?: any) => Promise<{ signedTxXdr: string }>;
}

export class AssembledTransaction<T> {
  server: rpc.Server;
  tx: Transaction;
  signed?: Transaction;

  constructor(
    public options: ClientOptions,
    public method: string,
    public args: any[],
    public parseResult: (val: any) => T,
  ) {
    this.server = new rpc.Server(options.rpcUrl, { allowHttp: options.allowHttp });
    const sourcePublicKey = options.publicKey || 'GAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAWHB';
    const contract = new Contract(options.contractId);
    
    const dummyAccount = new rpc.Server.Account(sourcePublicKey, '0');
    this.tx = new TransactionBuilder(dummyAccount, {
      fee: BASE_FEE,
      networkPassphrase: options.networkPassphrase,
    })
      .addOperation(contract.call(method, ...args))
      .setTimeout(30)
      .build();
  }

  async simulate(): Promise<T> {
    const sim = await this.server.simulateTransaction(this.tx);
    if (sim.error) throw new Error(sim.error);
    if (!sim.result) throw new Error('Simulation returned no result');
    return this.parseResult(sim.result.retval);
  }

  async sign(): Promise<this> {
    if (!this.options.signTransaction) {
      throw new Error('No signTransaction function provided');
    }
    const sim = await this.server.simulateTransaction(this.tx);
    if (sim.error) throw new Error(sim.error);
    const preparedTx = rpc.assembleTransaction(this.tx, sim).build();
    
    const { signedTxXdr } = await this.options.signTransaction(preparedTx.toXDR());
    this.signed = TransactionBuilder.fromXDR(signedTxXdr, this.options.networkPassphrase);
    return this;
  }

  async send(): Promise<T> {
    if (!this.signed) {
      throw new Error('Transaction not signed. Call sign() first.');
    }
    const sendResult = await this.server.sendTransaction(this.signed);
    if (sendResult.status === 'ERROR') {
      throw new Error(sendResult.errorResult?.toString() || 'Transaction submission failed.');
    }

    let getResult;
    for (let i = 0; i < 40; i++) {
      getResult = await this.server.getTransaction(sendResult.hash);
      if (getResult.status !== 'NOT_FOUND') break;
      await new Promise((r) => setTimeout(r, 1500));
    }

    if (!getResult || getResult.status === 'NOT_FOUND') {
      throw new Error('Transaction submitted but could not be confirmed in time.');
    }

    if (getResult.status === 'FAILED') {
      throw new Error('Transaction failed on-chain.');
    }

    return this.parseResult(getResult.returnValue);
  }

  async signAndSend(): Promise<T> {
    await this.sign();
    return this.send();
  }
}

export class Client {
  options: ClientOptions;

  constructor(options: ClientOptions) {
    this.options = options;
  }

  async balance({ user }: { user: string }): Promise<AssembledTransaction<bigint>> {
    return new AssembledTransaction(
      this.options,
      'balance',
      [nativeToScVal(Address.fromString(user))],
      (val) => scValToNative(val)
    );
  }

  async claim({ user, amount }: { user: string; amount: bigint }): Promise<AssembledTransaction<bigint>> {
    return new AssembledTransaction(
      this.options,
      'claim',
      [nativeToScVal(Address.fromString(user)), nativeToScVal(amount, { type: 'u64' })],
      (val) => scValToNative(val)
    );
  }

  async get_tier_for_rank({ rank, campaign_id }: { rank: bigint; campaign_id: bigint }): Promise<AssembledTransaction<bigint>> {
    return new AssembledTransaction(
      this.options,
      'get_tier_for_rank',
      [nativeToScVal(rank, { type: 'u64' }), nativeToScVal(campaign_id, { type: 'u64' })],
      (val) => scValToNative(val)
    );
  }

  async credit_by_rank({ admin, user, rank, campaign_id }: { admin: string; user: string; rank: bigint; campaign_id: bigint }): Promise<AssembledTransaction<bigint>> {
    return new AssembledTransaction(
      this.options,
      'credit_by_rank',
      [
        nativeToScVal(Address.fromString(admin)),
        nativeToScVal(Address.fromString(user)),
        nativeToScVal(rank, { type: 'u64' }),
        nativeToScVal(campaign_id, { type: 'u64' })
      ],
      (val) => scValToNative(val)
    );
  }
}
