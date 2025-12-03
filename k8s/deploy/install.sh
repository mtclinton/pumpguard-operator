#!/bin/bash
set -e

echo "🛡️  Installing PumpGuard Kubernetes Operator"
echo "============================================="

# Get the directory of this script
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
K8S_DIR="$(dirname "$SCRIPT_DIR")"

# Check if kubectl is available
if ! command -v kubectl &> /dev/null; then
    echo "❌ kubectl not found. Please install kubectl first."
    exit 1
fi

# Check cluster connection
if ! kubectl cluster-info &> /dev/null; then
    echo "❌ Cannot connect to Kubernetes cluster. Please check your kubeconfig."
    exit 1
fi

echo "✓ Connected to Kubernetes cluster"

# Install CRDs
echo ""
echo "📦 Installing Custom Resource Definitions..."
kubectl apply -f "$K8S_DIR/crds/pumpguard-crd.yaml"
echo "✓ CRDs installed"

# Install Operator
echo ""
echo "🚀 Deploying PumpGuard Operator..."
kubectl apply -f "$K8S_DIR/deploy/operator.yaml"
echo "✓ Operator deployed"

# Wait for operator to be ready
echo ""
echo "⏳ Waiting for operator to be ready..."
kubectl wait --for=condition=available --timeout=120s deployment/pumpguard-operator -n pumpguard-system

echo ""
echo "============================================="
echo "✅ PumpGuard Operator installed successfully!"
echo ""
echo "Next steps:"
echo "  1. Create a wallet secret:"
echo "     kubectl create secret generic pumpguard-wallet --from-literal=privateKey=YOUR_KEY"
echo ""
echo "  2. Deploy a PumpGuard instance:"
echo "     kubectl apply -f $K8S_DIR/examples/pumpguard-basic.yaml"
echo ""
echo "  3. Check status:"
echo "     kubectl get pumpguards"
echo ""
echo "  4. View logs:"
echo "     kubectl logs -f deployment/pumpguard-monitor-pumpguard"
echo ""

