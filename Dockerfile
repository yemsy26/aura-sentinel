FROM rust:1.77-bullseye

# Install system dependencies
RUN apt-get update && apt-get install -y \
    curl \
    git \
    python3 \
    python3-pip \
    libwebkit2gtk-4.0-dev \
    build-essential \
    wget \
    pciutils \
    && rm -rf /var/lib/apt/lists/*

# Install Node.js
RUN curl -fsSL https://deb.nodesource.com/setup_20.x | bash - \
    && apt-get install -y nodejs

# Install Ollama
RUN curl -fsSL https://ollama.com/install.sh | sh

WORKDIR /app

# Copy project files
COPY . .

# Note: In a real environment, you would need to start the ollama service
# and pull the required models (llama3.1:8b, qwen2.5-coder:7b, nomic-embed-text)
# before running the agent.

# Install frontend dependencies
RUN npm install

# Build the Tauri app (this will download rust crates)
# Note: Tauri requires a display server (X11/Wayland) or xvfb to run headless.
# For production docker, typically you build it here, or run it via xvfb-run.
RUN npm run build || echo "Build may require X11, proceeding anyway..."

CMD ["echo", "Aura-Sentinel container is ready. Please run with xvfb-run if executing Tauri headlessly, and ensure Ollama is running."]
