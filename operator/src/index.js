import Operator from '@dot-i/k8s-operator';
import { KubeConfig, CustomObjectsApi, CoreV1Api } from '@kubernetes/client-node';
import logger from '../../src/utils/logger.js';

const GROUP = 'pumpguard.fun';
const VERSION = 'v1';
const PUMPGUARD_PLURAL = 'pumpguards';
const TOKENMONITOR_PLURAL = 'tokenmonitors';

class PumpGuardOperator extends Operator {
  constructor() {
    super(logger);
    
    this.kubeConfig = new KubeConfig();
    this.kubeConfig.loadFromDefault();
    
    this.customApi = this.kubeConfig.makeApiClient(CustomObjectsApi);
    this.coreApi = this.kubeConfig.makeApiClient(CoreV1Api);
    
    // Track managed resources
    this.managedInstances = new Map();
  }

  async init() {
    // Register CRD watchers
    await this.watchResource(GROUP, VERSION, PUMPGUARD_PLURAL, async (event) => {
      await this.handlePumpGuardEvent(event);
    });

    await this.watchResource(GROUP, VERSION, TOKENMONITOR_PLURAL, async (event) => {
      await this.handleTokenMonitorEvent(event);
    });

    logger.info('OPERATOR', 'PumpGuard Kubernetes Operator initialized');
  }

  async handlePumpGuardEvent(event) {
    const { type, object } = event;
    const name = object.metadata.name;
    const namespace = object.metadata.namespace;
    const spec = object.spec || {};

    logger.info('OPERATOR', `PumpGuard event: ${type} ${namespace}/${name}`);

    try {
      switch (type) {
        case 'ADDED':
        case 'MODIFIED':
          await this.reconcilePumpGuard(object);
          break;
        case 'DELETED':
          await this.deletePumpGuard(namespace, name);
          break;
      }
    } catch (error) {
      logger.error('OPERATOR', `Error handling PumpGuard event: ${error.message}`);
      await this.updateStatus(namespace, name, PUMPGUARD_PLURAL, {
        phase: 'Error',
        conditions: [{
          type: 'Ready',
          status: 'False',
          reason: 'ReconcileError',
          message: error.message,
          lastTransitionTime: new Date().toISOString()
        }]
      });
    }
  }

  async reconcilePumpGuard(resource) {
    const name = resource.metadata.name;
    const namespace = resource.metadata.namespace;
    const spec = resource.spec || {};

    logger.info('OPERATOR', `Reconciling PumpGuard ${namespace}/${name}`);

    // Update status to Reconciling
    await this.updateStatus(namespace, name, PUMPGUARD_PLURAL, {
      phase: 'Reconciling',
      lastUpdated: new Date().toISOString()
    });

    // 1. Ensure ConfigMap exists
    await this.ensureConfigMap(namespace, name, spec);

    // 2. Ensure Deployment exists
    await this.ensureDeployment(namespace, name, spec);

    // 3. Ensure Service exists (for dashboard)
    if (spec.dashboard?.enabled !== false) {
      await this.ensureService(namespace, name, spec);
    }

    // 4. Ensure Ingress if configured
    if (spec.dashboard?.ingress?.enabled) {
      await this.ensureIngress(namespace, name, spec);
    }

    // Update status to Running
    await this.updateStatus(namespace, name, PUMPGUARD_PLURAL, {
      phase: 'Running',
      conditions: [{
        type: 'Ready',
        status: 'True',
        reason: 'Reconciled',
        message: 'PumpGuard instance is running',
        lastTransitionTime: new Date().toISOString()
      }],
      modules: {
        sniper: { running: spec.modules?.sniper?.enabled !== false },
        rugDetector: { running: spec.modules?.rugDetector?.enabled !== false },
        whaleWatcher: { running: spec.modules?.whaleWatcher?.enabled !== false }
      },
      lastUpdated: new Date().toISOString()
    });

    this.managedInstances.set(`${namespace}/${name}`, resource);
    logger.success('OPERATOR', `PumpGuard ${namespace}/${name} reconciled successfully`);
  }

  async ensureConfigMap(namespace, name, spec) {
    const configMapName = `${name}-config`;
    
    const configData = {
      'config.json': JSON.stringify({
        rpcUrl: spec.rpcUrl || 'https://api.mainnet-beta.solana.com',
        wsUrl: spec.wsUrl || 'wss://api.mainnet-beta.solana.com',
        modules: spec.modules || {},
        alerting: spec.alerting || {},
        dashboard: spec.dashboard || {}
      }, null, 2)
    };

    const configMap = {
      apiVersion: 'v1',
      kind: 'ConfigMap',
      metadata: {
        name: configMapName,
        namespace,
        labels: {
          'app.kubernetes.io/name': 'pumpguard',
          'app.kubernetes.io/instance': name,
          'app.kubernetes.io/managed-by': 'pumpguard-operator'
        },
        ownerReferences: [{
          apiVersion: `${GROUP}/${VERSION}`,
          kind: 'PumpGuard',
          name,
          uid: '', // Will be set by K8s
          controller: true,
          blockOwnerDeletion: true
        }]
      },
      data: configData
    };

    try {
      await this.coreApi.readNamespacedConfigMap(configMapName, namespace);
      await this.coreApi.replaceNamespacedConfigMap(configMapName, namespace, configMap);
      logger.info('OPERATOR', `ConfigMap ${configMapName} updated`);
    } catch (e) {
      if (e.statusCode === 404) {
        await this.coreApi.createNamespacedConfigMap(namespace, configMap);
        logger.info('OPERATOR', `ConfigMap ${configMapName} created`);
      } else {
        throw e;
      }
    }
  }

  async ensureDeployment(namespace, name, spec) {
    const deploymentName = `${name}-pumpguard`;
    const resources = spec.resources || {};

    const deployment = {
      apiVersion: 'apps/v1',
      kind: 'Deployment',
      metadata: {
        name: deploymentName,
        namespace,
        labels: {
          'app.kubernetes.io/name': 'pumpguard',
          'app.kubernetes.io/instance': name,
          'app.kubernetes.io/managed-by': 'pumpguard-operator'
        }
      },
      spec: {
        replicas: 1, // PumpGuard should only run 1 replica to avoid duplicate trades
        selector: {
          matchLabels: {
            'app.kubernetes.io/name': 'pumpguard',
            'app.kubernetes.io/instance': name
          }
        },
        template: {
          metadata: {
            labels: {
              'app.kubernetes.io/name': 'pumpguard',
              'app.kubernetes.io/instance': name
            }
          },
          spec: {
            containers: [{
              name: 'pumpguard',
              image: 'pumpguard/pumpguard:latest',
              ports: [{
                containerPort: spec.dashboard?.port || 3000,
                name: 'http'
              }],
              env: this.buildEnvVars(spec),
              volumeMounts: [{
                name: 'config',
                mountPath: '/app/config'
              }, {
                name: 'data',
                mountPath: '/app/data'
              }],
              resources: {
                requests: {
                  cpu: resources.requests?.cpu || '100m',
                  memory: resources.requests?.memory || '256Mi'
                },
                limits: {
                  cpu: resources.limits?.cpu || '500m',
                  memory: resources.limits?.memory || '512Mi'
                }
              },
              livenessProbe: {
                httpGet: {
                  path: '/api/stats',
                  port: 'http'
                },
                initialDelaySeconds: 30,
                periodSeconds: 10
              },
              readinessProbe: {
                httpGet: {
                  path: '/api/stats',
                  port: 'http'
                },
                initialDelaySeconds: 5,
                periodSeconds: 5
              }
            }],
            volumes: [{
              name: 'config',
              configMap: {
                name: `${name}-config`
              }
            }, {
              name: 'data',
              emptyDir: {}
            }]
          }
        }
      }
    };

    // Add wallet secret if configured
    if (spec.walletSecretRef) {
      deployment.spec.template.spec.containers[0].env.push({
        name: 'WALLET_PRIVATE_KEY',
        valueFrom: {
          secretKeyRef: {
            name: spec.walletSecretRef.name,
            key: spec.walletSecretRef.key || 'privateKey'
          }
        }
      });
    }

    // Add Telegram secret if configured
    if (spec.alerting?.telegram?.secretRef) {
      const telegramRef = spec.alerting.telegram.secretRef;
      deployment.spec.template.spec.containers[0].env.push({
        name: 'TELEGRAM_BOT_TOKEN',
        valueFrom: {
          secretKeyRef: {
            name: telegramRef.name,
            key: telegramRef.botTokenKey || 'botToken'
          }
        }
      }, {
        name: 'TELEGRAM_CHAT_ID',
        valueFrom: {
          secretKeyRef: {
            name: telegramRef.name,
            key: telegramRef.chatIdKey || 'chatId'
          }
        }
      });
    }

    try {
      const appsApi = this.kubeConfig.makeApiClient(await import('@kubernetes/client-node').then(m => m.AppsV1Api));
      await appsApi.readNamespacedDeployment(deploymentName, namespace);
      await appsApi.replaceNamespacedDeployment(deploymentName, namespace, deployment);
      logger.info('OPERATOR', `Deployment ${deploymentName} updated`);
    } catch (e) {
      if (e.statusCode === 404) {
        const appsApi = this.kubeConfig.makeApiClient(await import('@kubernetes/client-node').then(m => m.AppsV1Api));
        await appsApi.createNamespacedDeployment(namespace, deployment);
        logger.info('OPERATOR', `Deployment ${deploymentName} created`);
      } else {
        throw e;
      }
    }
  }

  buildEnvVars(spec) {
    const env = [
      { name: 'SOLANA_RPC_URL', value: spec.rpcUrl || 'https://api.mainnet-beta.solana.com' },
      { name: 'SOLANA_WS_URL', value: spec.wsUrl || 'wss://api.mainnet-beta.solana.com' },
      { name: 'DASHBOARD_PORT', value: String(spec.dashboard?.port || 3000) },
      { name: 'CONFIG_PATH', value: '/app/config/config.json' }
    ];

    // Sniper config
    if (spec.modules?.sniper) {
      env.push(
        { name: 'SNIPER_ENABLED', value: String(spec.modules.sniper.enabled !== false) },
        { name: 'MAX_BUY_AMOUNT_SOL', value: String(spec.modules.sniper.maxBuyAmountSol || 0.1) },
        { name: 'SLIPPAGE_PERCENT', value: String(spec.modules.sniper.slippagePercent || 15) }
      );
    }

    // Rug detector config
    if (spec.modules?.rugDetector) {
      env.push(
        { name: 'RUG_DETECTOR_ENABLED', value: String(spec.modules.rugDetector.enabled !== false) },
        { name: 'AUTO_SELL_ON_RUG', value: String(spec.modules.rugDetector.autoSellOnRug !== false) },
        { name: 'LP_REMOVAL_THRESHOLD_PERCENT', value: String(spec.modules.rugDetector.lpRemovalThresholdPercent || 50) }
      );
    }

    // Whale watcher config
    if (spec.modules?.whaleWatcher) {
      env.push(
        { name: 'WHALE_WATCHER_ENABLED', value: String(spec.modules.whaleWatcher.enabled !== false) },
        { name: 'WHALE_THRESHOLD_SOL', value: String(spec.modules.whaleWatcher.whaleThresholdSol || 50) }
      );
    }

    return env;
  }

  async ensureService(namespace, name, spec) {
    const serviceName = `${name}-dashboard`;
    const port = spec.dashboard?.port || 3000;

    const service = {
      apiVersion: 'v1',
      kind: 'Service',
      metadata: {
        name: serviceName,
        namespace,
        labels: {
          'app.kubernetes.io/name': 'pumpguard',
          'app.kubernetes.io/instance': name,
          'app.kubernetes.io/managed-by': 'pumpguard-operator'
        }
      },
      spec: {
        type: 'ClusterIP',
        ports: [{
          port,
          targetPort: 'http',
          protocol: 'TCP',
          name: 'http'
        }],
        selector: {
          'app.kubernetes.io/name': 'pumpguard',
          'app.kubernetes.io/instance': name
        }
      }
    };

    try {
      await this.coreApi.readNamespacedService(serviceName, namespace);
      await this.coreApi.replaceNamespacedService(serviceName, namespace, service);
      logger.info('OPERATOR', `Service ${serviceName} updated`);
    } catch (e) {
      if (e.statusCode === 404) {
        await this.coreApi.createNamespacedService(namespace, service);
        logger.info('OPERATOR', `Service ${serviceName} created`);
      } else {
        throw e;
      }
    }
  }

  async ensureIngress(namespace, name, spec) {
    const ingressName = `${name}-ingress`;
    const host = spec.dashboard?.ingress?.host || `${name}.pumpguard.local`;

    const ingress = {
      apiVersion: 'networking.k8s.io/v1',
      kind: 'Ingress',
      metadata: {
        name: ingressName,
        namespace,
        labels: {
          'app.kubernetes.io/name': 'pumpguard',
          'app.kubernetes.io/instance': name,
          'app.kubernetes.io/managed-by': 'pumpguard-operator'
        },
        annotations: {
          'kubernetes.io/ingress.class': 'nginx'
        }
      },
      spec: {
        rules: [{
          host,
          http: {
            paths: [{
              path: '/',
              pathType: 'Prefix',
              backend: {
                service: {
                  name: `${name}-dashboard`,
                  port: { number: spec.dashboard?.port || 3000 }
                }
              }
            }]
          }
        }]
      }
    };

    if (spec.dashboard?.ingress?.tls) {
      ingress.spec.tls = [{
        hosts: [host],
        secretName: `${name}-tls`
      }];
    }

    try {
      const networkingApi = this.kubeConfig.makeApiClient(await import('@kubernetes/client-node').then(m => m.NetworkingV1Api));
      await networkingApi.readNamespacedIngress(ingressName, namespace);
      await networkingApi.replaceNamespacedIngress(ingressName, namespace, ingress);
      logger.info('OPERATOR', `Ingress ${ingressName} updated`);
    } catch (e) {
      if (e.statusCode === 404) {
        const networkingApi = this.kubeConfig.makeApiClient(await import('@kubernetes/client-node').then(m => m.NetworkingV1Api));
        await networkingApi.createNamespacedIngress(namespace, ingress);
        logger.info('OPERATOR', `Ingress ${ingressName} created`);
      } else {
        throw e;
      }
    }
  }

  async deletePumpGuard(namespace, name) {
    logger.info('OPERATOR', `Deleting PumpGuard ${namespace}/${name}`);
    this.managedInstances.delete(`${namespace}/${name}`);
    // Kubernetes will garbage collect owned resources via ownerReferences
  }

  async handleTokenMonitorEvent(event) {
    const { type, object } = event;
    const name = object.metadata.name;
    const namespace = object.metadata.namespace;

    logger.info('OPERATOR', `TokenMonitor event: ${type} ${namespace}/${name}`);

    // TokenMonitor resources are handled by the PumpGuard instance
    // This just updates the status based on the linked PumpGuard
  }

  async updateStatus(namespace, name, plural, status) {
    try {
      const current = await this.customApi.getNamespacedCustomObject(
        GROUP, VERSION, namespace, plural, name
      );
      
      const updated = {
        ...current.body,
        status: {
          ...current.body.status,
          ...status
        }
      };

      await this.customApi.replaceNamespacedCustomObjectStatus(
        GROUP, VERSION, namespace, plural, name, updated
      );
    } catch (e) {
      logger.error('OPERATOR', `Failed to update status: ${e.message}`);
    }
  }
}

// Start the operator
const operator = new PumpGuardOperator();

(async () => {
  try {
    await operator.start();
    logger.success('OPERATOR', 'PumpGuard Operator started successfully');
  } catch (error) {
    logger.error('OPERATOR', `Failed to start operator: ${error.message}`);
    process.exit(1);
  }
})();

export default operator;

