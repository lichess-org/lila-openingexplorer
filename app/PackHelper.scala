package lila.openingexplorer

import java.io.{ InputStream, OutputStream }
import chess.format.Uci
import chess.{ Pos, Role }

trait PackHelper {

  protected def writeUint(stream: OutputStream, v: Long) = {
    var value = v
    while (value > 127) {
      stream.write(((value & 127) | 128).toInt)
      value >>= 7
    }
    stream.write((value & 127).toInt)
  }

  protected def readUint(stream: InputStream): Long = {
    var value: Long = 0
    var i: Int      = 0
    var byte: Int   = 0

    do {
      byte = stream.read()
      value |= (byte.toLong & 127) << (7 * i)
      i += 1
    } while ((byte & 128) != 0)

    value
  }

  protected def writeUint16(stream: OutputStream, v: Int) = {
    stream.write(0xff & (v >> 8))
    stream.write(0xff & v)
  }

  protected def readUint16(stream: InputStream): Int =
    stream.read() << 8 | stream.read()

  protected def writeUint48(stream: OutputStream, v: Long) = {
    stream.write((0xff & (v >> 40)).toInt)
    stream.write((0xff & (v >> 32)).toInt)
    stream.write((0xff & (v >> 24)).toInt)
    stream.write((0xff & (v >> 16)).toInt)
    stream.write((0xff & (v >> 8)).toInt)
    stream.write((0xff & v).toInt)
  }

  protected def readUint48(stream: InputStream): Long =
    stream.read.toLong << 40 | stream.read.toLong << 32 |
      stream.read.toLong << 24 | stream.read.toLong << 16 |
      stream.read.toLong << 8 | stream.read.toLong

  protected def writeUci(stream: OutputStream, move: Uci.Move): Unit =
    writeUint16(
      stream,
      Pos.all.indexOf(move.orig) |
        Pos.all.indexOf(move.dest) << 6 |
        move.promotion.fold(0)(r => (Role.allPromotable.indexOf(r)) + 1) << 12
    )

  protected def writeUci(stream: OutputStream, drop: Uci.Drop): Unit = {
    val dest = Pos.all.indexOf(drop.pos)
    writeUint16(stream, dest | dest << 6 | (Role.all.indexOf(drop.role) + 1) << 12)
  }

  protected def writeUci(stream: OutputStream, move: Either[Uci.Move, Uci.Drop]): Unit =
    move.fold(writeUci(stream, _), writeUci(stream, _))

  protected def readUci(stream: InputStream): Either[Uci.Move, Uci.Drop] = {
    val enc  = readUint16(stream)
    val orig = Pos.all(enc & 63)
    val dest = Pos.all((enc >> 6) & 63)
    if (orig == dest) {
      Right(new Uci.Drop(Role.all((enc >> 12) - 1), dest))
    } else {
      val role = if ((enc >> 12) != 0) Some(Role.allPromotable((enc >> 12) - 1)) else None
      Left(new Uci.Move(orig, dest, role))
    }
  }
}
