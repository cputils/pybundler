import app.runner
from vendor.tool import helper  # bundle


def run():
    return app.runner.run() + helper()


if __name__ == "__main__":
    print(run())
