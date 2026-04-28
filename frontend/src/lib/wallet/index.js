import { WalletManager } from './WalletManager.js';
import { FreighterProvider } from './FreighterProvider.js';

const walletManager = new WalletManager();

walletManager.registerProvider(new FreighterProvider());

export { walletManager, WalletManager, FreighterProvider };
export { WalletProvider } from './WalletProvider.js';
