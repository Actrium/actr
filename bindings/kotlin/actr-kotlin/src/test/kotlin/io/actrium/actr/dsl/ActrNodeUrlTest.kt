package io.actrium.actr.dsl

import io.actrium.actr.ActrType
import io.actrium.actr.DynamicWorkload
import io.actrium.actr.NoHandle
import kotlinx.coroutines.test.runTest
import java.net.URL
import kotlin.test.Test
import kotlin.test.assertFailsWith
import kotlin.test.assertTrue

class ActrNodeUrlTest {
    @Test
    fun `package URL overload rejects non-file config URL before native creation`() =
        runTest {
            val error =
                assertFailsWith<IllegalArgumentException> {
                    ActrNode.fromPackageFile(
                        configURL = URL("https://example.com/actr.toml"),
                        packageURL = URL("file:/tmp/app.actr"),
                    )
                }

            assertTrue("configURL must be a file URL" in error.message.orEmpty())
        }

    @Test
    fun `top-level package URL overload rejects non-file package URL before native creation`() =
        runTest {
            val error =
                assertFailsWith<IllegalArgumentException> {
                    createActrNode(
                        configURL = URL("file:/tmp/actr.toml"),
                        packageURL = URL("https://example.com/app.actr"),
                    )
                }

            assertTrue("packageURL must be a file URL" in error.message.orEmpty())
        }

    @Test
    fun `linked URL overload rejects non-file config URL before native creation`() =
        runTest {
            val error =
                assertFailsWith<IllegalArgumentException> {
                    ActrNode.linked(
                        configURL = URL("https://example.com/actr.toml"),
                        actorType = ActrType("acme", "Client", "1.0.0"),
                        workload = DynamicWorkload(NoHandle),
                    )
                }

            assertTrue("config URL must be a file URL" in error.message.orEmpty())
        }
}
