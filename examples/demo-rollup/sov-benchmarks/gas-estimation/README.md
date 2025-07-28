## Introduction
This folder contains constant benchmarking utilities that can be used to generate values of gas cost for the rollups using the Sovereign SDK.

## Usage
It is recommended to use a virtual environment to run the optimization script.

### Installation
The virtual environment can be setup using the following command:
```bash
cd gas-estimation
python3 -m venv venv
source venv/bin/activate
```

You may install the file dependencies using the following command:
```bash
pip install -r requirements.txt 
```

### Running the scripts

`process_metrics.py`: Transposes the constants CSV file so that constants are represented as columns instead of rows. Concatenates files generated from different benchmark runs.

```bash
python3 process_metrics.py 
```
