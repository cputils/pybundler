import app.runner
from vendor.tool import helper


def run():
    return app.runner.run() + helper()


if __name__ == "__main__":
    print(run())
