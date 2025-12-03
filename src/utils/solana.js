import { Connection, PublicKey, LAMPORTS_PER_SOL } from '@solana/web3.js';
import config from '../config.js';
import logger from './logger.js';

class SolanaService {
  constructor() {
    this.connection = new Connection(config.rpcUrl, {
      commitment: 'confirmed',
      wsEndpoint: config.wsUrl
    });
    
    this.pumpProgramId = new PublicKey(config.pumpProgramId);
    logger.info('SOLANA', 'Connected to Solana RPC (monitor-only mode)');
  }
  
  async getBalance(pubkey) {
    if (!pubkey) return 0;
    const key = new PublicKey(pubkey);
    const balance = await this.connection.getBalance(key);
    return balance / LAMPORTS_PER_SOL;
  }
  
  async subscribeToProgram(programId, callback) {
    const id = this.connection.onProgramAccountChange(
      new PublicKey(programId),
      callback,
      'confirmed'
    );
    logger.info('SOLANA', `Subscribed to program: ${programId}`);
    return id;
  }
  
  async subscribeToPumpLogs(callback) {
    const id = this.connection.onLogs(
      this.pumpProgramId,
      callback,
      'confirmed'
    );
    logger.info('SOLANA', 'Subscribed to pump.fun logs');
    return id;
  }
  
  async getTransaction(signature) {
    return this.connection.getParsedTransaction(signature, {
      maxSupportedTransactionVersion: 0
    });
  }
  
  async getRecentTransactions(address, limit = 20) {
    const pubkey = new PublicKey(address);
    const signatures = await this.connection.getSignaturesForAddress(pubkey, { limit });
    return signatures;
  }
  
  async getAccountInfo(address) {
    return this.connection.getAccountInfo(new PublicKey(address));
  }
  
  shortenAddress(address, chars = 4) {
    if (!address) return 'unknown';
    return `${address.slice(0, chars)}...${address.slice(-chars)}`;
  }
}

export const solanaService = new SolanaService();
export default solanaService;
