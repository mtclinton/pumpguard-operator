import chalk from 'chalk';

const timestamp = () => new Date().toISOString();

export const logger = {
  info: (module, message, data = null) => {
    console.log(
      chalk.gray(`[${timestamp()}]`),
      chalk.blue(`[${module}]`),
      chalk.white(message),
      data ? chalk.gray(JSON.stringify(data)) : ''
    );
  },
  
  success: (module, message, data = null) => {
    console.log(
      chalk.gray(`[${timestamp()}]`),
      chalk.green(`[${module}]`),
      chalk.greenBright(message),
      data ? chalk.gray(JSON.stringify(data)) : ''
    );
  },
  
  warn: (module, message, data = null) => {
    console.log(
      chalk.gray(`[${timestamp()}]`),
      chalk.yellow(`[${module}]`),
      chalk.yellowBright(message),
      data ? chalk.gray(JSON.stringify(data)) : ''
    );
  },
  
  error: (module, message, data = null) => {
    console.log(
      chalk.gray(`[${timestamp()}]`),
      chalk.red(`[${module}]`),
      chalk.redBright(message),
      data ? chalk.gray(JSON.stringify(data)) : ''
    );
  },
  
  alert: (module, message, data = null) => {
    console.log(
      chalk.gray(`[${timestamp()}]`),
      chalk.magenta(`[${module}]`),
      chalk.magentaBright('🚨 ' + message),
      data ? chalk.gray(JSON.stringify(data)) : ''
    );
  },
  
  whale: (message, data = null) => {
    console.log(
      chalk.gray(`[${timestamp()}]`),
      chalk.cyan('[WHALE]'),
      chalk.cyanBright('🐋 ' + message),
      data ? chalk.gray(JSON.stringify(data)) : ''
    );
  },
  
  snipe: (message, data = null) => {
    console.log(
      chalk.gray(`[${timestamp()}]`),
      chalk.hex('#FF6B6B')('[SNIPER]'),
      chalk.hex('#FF6B6B')('🎯 ' + message),
      data ? chalk.gray(JSON.stringify(data)) : ''
    );
  },
  
  rug: (message, data = null) => {
    console.log(
      chalk.gray(`[${timestamp()}]`),
      chalk.hex('#FF0000')('[RUG ALERT]'),
      chalk.bgRed.white(' ⚠️  ' + message + ' '),
      data ? chalk.gray(JSON.stringify(data)) : ''
    );
  }
};

export default logger;

