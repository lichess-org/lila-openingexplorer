#include <iostream>
#include <string>

#include <kcpolydb.h>

kyotocabinet::PolyDB *out;

int main(int argc, char **argv) {
  if (argc != 3) {
    std::cout << "Usage: convert <in> <out>" << std::endl;
    std::cout << "Copies records from in to out." << std::endl;
  }

  std::cout << "Waiting for read lock ..." << std::endl;
  kyotocabinet::PolyDB in;
  if (!in.open(argv[1], kyotocabinet::PolyDB::OREADER)) {
    std::cout << "Could not open source database." << std::endl;
    return 1;
  }

  std::cout << "Waiting for write lock ..." << std::endl;
  out = new kyotocabinet::PolyDB();
  if (!out->open(argv[2], kyotocabinet::PolyDB::OWRITER | kyotocabinet::PolyDB::OCREATE)) {
    std::cout << "Could not open target database." << std::endl;
    return 1;
  }

  std::cout << "Copying ..." << std::endl;

  class VisitorImpl : public kyotocabinet::DB::Visitor {
    long total = 0;

    const char *visit_full(const char *kbuf, size_t ksiz, const char *vbuf, size_t vsiz, size_t *sp) {
      out->set(kbuf, ksiz, vbuf, vsiz);
      total++;
      if (total % 100000 == 0) {
        std::cerr << ".";
      }
      return NOP;
    }

    const char *visit_empty(const char *kbuf, size_t ksiz, size_t *sp) {
      return NOP;
    }
  } visitor;

  if (!in.iterate(&visitor, false)) {
    std::cout << "Failed to iterate." << std::endl;
    return 1;
  }

  std::cerr << std::endl;
  std::cout << "Done.";
  in.close();
  out->close();
  return 0;
}
