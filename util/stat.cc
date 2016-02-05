#include <iostream>
#include <string>

#include <kcpolydb.h>

long estimate_a(long *pack) {
  return (
    4 + pack[0] * 8 +
    4 + 1 + pack[1] * 8 * 5 +
    4 + 1 + pack[2] * 8 * 5 + 3 * 11 * 1 +
    4 + 1 + pack[3] * 8 * 5 + 3 * 11 * 2 +
    4 + 1 + pack[4] * 8 * 5 + 3 * 11 * 4 +
    4 + 1 + pack[5] * 8 * 5 + 3 * 11 * 6
  );
}

long estimate_b(long *pack) {
  return (
    4 + pack[0] * 9 +
    4 + 1 + pack[1] * 9 * 5 +
    4 + 1 + pack[2] * 9 * 5 + 4 * 2 * 3 * 8 * 1 +
    4 + 1 + pack[3] * 9 * 5 + 4 * 2 * 3 * 8 * 2 +
    4 + 1 + pack[4] * 9 * 5 + 4 * 2 * 3 * 8 * 4 +
    4 + 1 + pack[5] * 9 * 5 + 4 * 2 * 3 * 8 * 6
  );
}

int main(int argc, char **argv) {
    if (argc <= 1) {
      std::cout << "Usage: stat <dbfile.kct>" << std::endl;
      return 1;
    }

    kyotocabinet::TreeDB db;

    if (!db.open(argv[1], kyotocabinet::TreeDB::OREADER)) {
      std::cout << "Could not open database." << std::endl;
      return 2;
    }

    std::auto_ptr<kyotocabinet::TreeDB::Cursor> cur(db.cursor());
    cur->jump();

    std::string key, value;

    long pack[] = {0, 0, 0, 0, 0, 0};

    while (cur->get(&key, &value, true)) {
        if (value.size() == 8) {
            pack[0]++;
        } else {
            pack[value.at(0)]++;
        }
    }

    for (int i = 0; i < 5; i++) {
        std::cout << "Pack format " << i << ": " << pack[i] << " nodes " << std::endl;
    }

    std::cout << "Unique positions: " << (pack[0] + pack[1] + pack[2] + pack[3] + pack[4] + pack[5]) << std::endl;

    std::cout << std::endl;

    std::cout << "Scheme A: " << estimate_a(pack) << " bytes" << std::endl;
    std::cout << "Scheme B: " << estimate_b(pack) << " bytes" << std::endl;
    std::cout << "B/A: " << ((double)estimate_b(pack)/estimate_a(pack)) << std::endl;

    return 0;
}
