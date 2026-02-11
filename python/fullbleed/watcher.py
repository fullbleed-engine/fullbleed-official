# SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Fullbleed-Commercial
import sys
import time
from pathlib import Path
from watchdog.observers import Observer
from watchdog.events import FileSystemEventHandler
from .builder import cmd_build
import argparse

class BuildEventHandler(FileSystemEventHandler):
    def __init__(self, args, delay=0.5):
        self.args = args
        self.delay = delay
        self.last_build = 0

    def on_modified(self, event):
        if event.is_directory:
            return
            
        # Ignore hidden files, build artifacts
        if "/." in event.src_path or "\\." in event.src_path:
            return
        if "dist" in event.src_path or "output" in event.src_path:
            return
            
        # Debounce
        now = time.time()
        if now - self.last_build < self.delay:
            return
            
        print(f"[watch] Change detected in {event.src_path}...")
        try:
             # Re-run build command
             # We might need to reload modules if python code changes?
             # For now, simplistic re-execution of build logic
             cmd_build(self.args)
        except Exception as e:
             print(f"[error] Build failed: {e}")
        
        self.last_build = now

def cmd_watch(args):
    """Watch project directory and trigger builds on change."""
    path = Path(args.config).parent if args.config else Path.cwd()
    
    print(f"[watch] Watching {path} for changes...")
    
    # Initial build
    try:
        cmd_build(args)
    except Exception as e:
        print(f"[error] Initial build failed: {e}")

    event_handler = BuildEventHandler(args)
    observer = Observer()
    observer.schedule(event_handler, str(path), recursive=True)
    observer.start()
    
    try:
        while True:
            time.sleep(1)
    except KeyboardInterrupt:
        observer.stop()
    observer.join()
